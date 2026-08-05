/*
    Copyright 2026 Sojan James

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

//! Dynamic Types: construct a topic type's structure at runtime instead of generating a Rust
//! struct at compile time via `cdds_derive`. See docs/design/dynamic-types.md for the full
//! design rationale.
//!
//! MVP scope only: flat structs, FINAL extensibility, primitive members + unbounded strings.
//! No nested structs, arrays, sequences, unions, or enums/bitmasks yet.
//!
//! Dynamic-type topics don't go through this crate's serdes.rs plugin at all - they use
//! CycloneDDS's own built-in default sertype (the same one idlc-generated C structs use),
//! driven by a dds_topic_descriptor_t CycloneDDS synthesizes from the type definition. That
//! sertype expects samples as raw, C-struct-layout memory: a buffer of `m_size` bytes,
//! aligned to `m_align`, with each field at the byte offset CycloneDDS's marshalling
//! bytecode (`m_ops`) says it's at. `layout::parse` walks that bytecode to recover the
//! offsets - see its module comment for why that's the only safe way to get them.

use cyclonedds_sys::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Arc;

use crate::dds_listener::DdsListener;
use crate::dds_participant::DdsParticipant;
use crate::dds_qos::DdsQos;
use crate::Entity;
pub use cyclonedds_sys::DdsEntity;

/// The subset of `dds_dynamic_type_kind_t` this MVP supports: primitives plus unbounded
/// strings. No nested/collection/union kinds yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicKind {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Char8,
    String,
}

impl DynamicKind {
    fn dds_kind(self) -> dds_dynamic_type_kind {
        match self {
            DynamicKind::Bool => dds_dynamic_type_kind::DDS_DYNAMIC_BOOLEAN,
            DynamicKind::I8 => dds_dynamic_type_kind::DDS_DYNAMIC_INT8,
            DynamicKind::U8 => dds_dynamic_type_kind::DDS_DYNAMIC_UINT8,
            DynamicKind::I16 => dds_dynamic_type_kind::DDS_DYNAMIC_INT16,
            DynamicKind::U16 => dds_dynamic_type_kind::DDS_DYNAMIC_UINT16,
            DynamicKind::I32 => dds_dynamic_type_kind::DDS_DYNAMIC_INT32,
            DynamicKind::U32 => dds_dynamic_type_kind::DDS_DYNAMIC_UINT32,
            DynamicKind::I64 => dds_dynamic_type_kind::DDS_DYNAMIC_INT64,
            DynamicKind::U64 => dds_dynamic_type_kind::DDS_DYNAMIC_UINT64,
            DynamicKind::F32 => dds_dynamic_type_kind::DDS_DYNAMIC_FLOAT32,
            DynamicKind::F64 => dds_dynamic_type_kind::DDS_DYNAMIC_FLOAT64,
            DynamicKind::Char8 => dds_dynamic_type_kind::DDS_DYNAMIC_CHAR8,
            DynamicKind::String => dds_dynamic_type_kind::DDS_DYNAMIC_STRING8,
        }
    }
}

mod layout {
    //! Parses `dds_topic_descriptor_t.m_ops` to recover each member's byte offset.
    //!
    //! CycloneDDS computes `m_ops`/`m_size`/`m_align` for us from the dynamic type
    //! descriptor (the same bytecode `idlc` emits for statically-compiled types). We could
    //! in principle predict field offsets ourselves using ordinary natural-alignment rules,
    //! but that requires our layout algorithm to provably match CycloneDDS's, forever, with
    //! no compiler to catch a divergence - just silent memory corruption (we'd write a field
    //! at the offset *we* computed; CycloneDDS would read/serialize it from the offset *it*
    //! computed). Parsing `m_ops` instead uses the numbers CycloneDDS itself produced, so
    //! there's no algorithm to keep in sync - see docs/design/dynamic-types.md §3.1.
    //!
    //! Format reference: dds_opcodes.h (a public header, split out specifically so language
    //! bindings can do this). MVP only handles the flat-FINAL-struct subset: a straight list
    //! of `ADR` instructions (one per member, in declaration order) terminated by `RTS`, with
    //! no JSR/PLC/DLC/UNI/SEQ/ARR - those only appear for extensibility other than FINAL, or
    //! for member kinds outside DynamicKind's MVP subset.

    use super::DynamicKind;
    use cyclonedds_sys::*;

    // Not bound in cyclonedds-sys: these are simple, stable bitmask #defines from the public
    // dds_opcodes.h, not function/type signatures that could break ABI, so they're hardcoded
    // here rather than plumbed through bindgen.
    const DDS_OP_MASK: u32 = 0xff000000;
    const DDS_OP_TYPE_MASK: u32 = 0x007f0000;

    #[derive(Debug, Clone)]
    pub(crate) struct FieldLayout {
        pub(crate) name: String,
        pub(crate) offset: u32,
        pub(crate) kind: DynamicKind,
        pub(crate) is_key: bool,
    }

    fn expected_typecode(kind: DynamicKind) -> u32 {
        (match kind {
            DynamicKind::Bool => dds_stream_typecode_primary_DDS_OP_TYPE_BLN,
            DynamicKind::I8 | DynamicKind::U8 | DynamicKind::Char8 => {
                dds_stream_typecode_primary_DDS_OP_TYPE_1BY
            }
            DynamicKind::I16 | DynamicKind::U16 => dds_stream_typecode_primary_DDS_OP_TYPE_2BY,
            DynamicKind::I32 | DynamicKind::U32 | DynamicKind::F32 => {
                dds_stream_typecode_primary_DDS_OP_TYPE_4BY
            }
            DynamicKind::I64 | DynamicKind::U64 | DynamicKind::F64 => {
                dds_stream_typecode_primary_DDS_OP_TYPE_8BY
            }
            DynamicKind::String => dds_stream_typecode_primary_DDS_OP_TYPE_STR,
        }) as u32
    }

    /// `members` is the builder's own ordered (name, kind, is_key) list - the source of
    /// truth for names (m_ops carries no field names), zipped positionally against the
    /// offsets parsed out of `m_ops`.
    pub(crate) fn parse(
        m_ops: &[u32],
        members: &[(String, DynamicKind, bool)],
    ) -> Result<Vec<FieldLayout>, DDSError> {
        let mut fields = Vec::with_capacity(members.len());
        let mut i = 0usize;
        let mut member_idx = 0usize;

        while i < m_ops.len() {
            let op = m_ops[i];
            let opcode = op & DDS_OP_MASK;

            if opcode == dds_stream_opcode_DDS_OP_RTS {
                break;
            }
            if opcode != dds_stream_opcode_DDS_OP_ADR {
                // Anything else (JSR/PLC/DLC/...) means this isn't the flat-FINAL,
                // primitive-only shape the MVP parser understands.
                return Err(DDSError::Unsupported);
            }
            let Some((name, kind, is_key)) = members.get(member_idx) else {
                return Err(DDSError::Unsupported);
            };

            let typecode = op & DDS_OP_TYPE_MASK;
            if typecode != expected_typecode(*kind) {
                // Confirms the op-stream is walking in the order we assumed (declaration
                // order matching m_ops order) - if CycloneDDS ever produced a different
                // order for FINAL structs, this catches it as an error instead of silently
                // mislabelling an offset.
                return Err(DDSError::Unsupported);
            }

            let offset = *m_ops.get(i + 1).ok_or(DDSError::Unsupported)?;
            fields.push(FieldLayout {
                name: name.clone(),
                offset,
                kind: *kind,
                is_key: *is_key,
            });

            member_idx += 1;
            i += 2; // [ADR, type, subtype, flags] [offset] - no extra operands for any MVP kind
        }

        if member_idx != members.len() {
            return Err(DDSError::Unsupported);
        }
        Ok(fields)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Hand-built m_ops matching the documented [ADR, type, 0, flags] [offset] shape,
        // exactly as CycloneDDS would emit for a flat FINAL struct with these two members -
        // validates the parser without needing a live participant.
        #[test]
        fn parses_flat_final_struct() {
            const DDS_OP_ADR: u32 = 0x01 << 24;
            const DDS_OP_RTS: u32 = 0x00 << 24;
            const DDS_OP_TYPE_4BY: u32 = 0x03 << 16;
            const DDS_OP_TYPE_STR: u32 = 0x05 << 16;
            const DDS_OP_FLAG_KEY: u32 = 1;

            let m_ops = vec![
                DDS_OP_ADR | DDS_OP_TYPE_4BY | DDS_OP_FLAG_KEY,
                0, // id: u32 at offset 0
                DDS_OP_ADR | DDS_OP_TYPE_STR,
                8, // label: string at offset 8 (after 4-byte id + padding to pointer align)
                DDS_OP_RTS,
            ];
            let members = vec![
                ("id".to_string(), DynamicKind::U32, true),
                ("label".to_string(), DynamicKind::String, false),
            ];

            let fields = parse(&m_ops, &members).expect("parse succeeds");
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "id");
            assert_eq!(fields[0].offset, 0);
            assert!(fields[0].is_key);
            assert_eq!(fields[1].name, "label");
            assert_eq!(fields[1].offset, 8);
            assert!(!fields[1].is_key);
        }

        #[test]
        fn rejects_kind_mismatch() {
            const DDS_OP_ADR: u32 = 0x01 << 24;
            const DDS_OP_TYPE_STR: u32 = 0x05 << 16;

            // m_ops says STR, our tracked member list says U32 - must not silently accept.
            let m_ops = vec![DDS_OP_ADR | DDS_OP_TYPE_STR, 0];
            let members = vec![("id".to_string(), DynamicKind::U32, false)];
            assert!(parse(&m_ops, &members).is_err());
        }

        #[test]
        fn rejects_unsupported_opcode() {
            const DDS_OP_JSR: u32 = 0x02 << 24;
            let m_ops = vec![DDS_OP_JSR, 0];
            let members = vec![("id".to_string(), DynamicKind::U32, false)];
            assert!(parse(&m_ops, &members).is_err());
        }
    }
}

/// A registered, resolved dynamic type: `m_size`/`m_align` plus the parsed field layout
/// (name -> offset/kind/is_key). Cheap to clone - shared via `Arc` between a topic and every
/// writer/reader/sample built from it, none of which need anything from the transient
/// `dds_dynamic_type_t`/`dds_topic_descriptor_t` used to construct this (both are freed
/// inside `DynamicTypeBuilder::build`, per the C API's own documented lifecycle - see
/// docs/design/dynamic-types.md §2.2).
struct DynamicLayout {
    type_name: String,
    size: u32,
    align: u32,
    fields: Vec<layout::FieldLayout>,
    field_index: HashMap<String, usize>,
    // Kept alive (not freed/unref'd at the end of DynamicTypeBuilder::build()) because
    // DdsDynamicTopic::create() needs type_info to call dds_create_topic_descriptor again for
    // the actual topic creation - dds_topic_descriptor_t itself is transient/short-lived per
    // the C API's own usage pattern (freed right after dds_create_topic returns), but the
    // registered type backing it can and should outlive any single topic creation, since the
    // same DdsDynamicType may back more than one topic.
    //
    // Both fields must stay alive together: empirically, unref'ing dtype right after
    // registering makes a *later* dds_create_topic_descriptor call using type_info fail with
    // PRECONDITION_NOT_MET, i.e. the type library does not keep a type resolvable purely from
    // type_info once its creator's last dtype reference is dropped.
    type_info: *mut ddsi_typeinfo,
    dtype: dds_dynamic_type_t,
}

// type_info/dtype are opaque, read-only-after-registration handles into cyclone's type
// library; nothing here is thread-affine.
unsafe impl Send for DynamicLayout {}
unsafe impl Sync for DynamicLayout {}

impl Drop for DynamicLayout {
    fn drop(&mut self) {
        unsafe {
            dds_free_typeinfo(self.type_info);
            dds_dynamic_type_unref(&mut self.dtype);
        }
    }
}

#[derive(Clone)]
pub struct DdsDynamicType(Arc<DynamicLayout>);

impl DdsDynamicType {
    pub fn type_name(&self) -> &str {
        &self.0.type_name
    }

    fn field(&self, name: &str) -> Option<&layout::FieldLayout> {
        self.0.field_index.get(name).map(|&i| &self.0.fields[i])
    }

    /// Whether `name` was marked as a key field when this type was built. Returns false for
    /// an unknown field name.
    pub fn is_key(&self, name: &str) -> bool {
        self.field(name).is_some_and(|f| f.is_key)
    }

    fn alloc_layout(&self) -> std::alloc::Layout {
        std::alloc::Layout::from_size_align(self.0.size as usize, self.0.align as usize)
            .expect("CycloneDDS-provided size/align are always valid")
    }
}

/// Builds a flat, FINAL-extensibility struct type at runtime. See the module doc for the
/// current (MVP) scope: primitive members and unbounded strings only, no nesting.
pub struct DynamicTypeBuilder {
    type_name: String,
    members: Vec<(String, DynamicKind, bool)>,
}

/// Builds the `dds_dynamic_type_spec_t` for a member of the given kind. True scalar
/// primitives (bool, integers, floats, char8) can be referenced inline via a
/// DDS_DYNAMIC_TYPE_KIND_PRIMITIVE spec - but despite being listed in the same
/// `dds_dynamic_type_kind_t` enum, CycloneDDS does *not* accept DDS_DYNAMIC_STRING8 that way
/// (confirmed empirically: dds_dynamic_type_add_member returns BAD_PARAMETER for a PRIMITIVE
/// spec of kind STRING8). Every other kind in dds_dynamic_type.c's own test suite that isn't
/// a bare scalar - structs, unions, aliases, arrays, sequences, and evidently strings too -
/// is instead created as its own standalone dds_dynamic_type_t first and referenced via a
/// DDS_DYNAMIC_TYPE_KIND_DEFINITION spec. Once dds_dynamic_type_add_member succeeds, the
/// parent struct takes over the new type's ownership (per dds_dynamic_type_add_member's own
/// doc comment), so it's *not* separately unref'd here.
unsafe fn build_member_type_spec(
    participant_entity: dds_entity_t,
    kind: DynamicKind,
) -> Result<dds_dynamic_type_spec_t, DDSError> {
    match kind {
        DynamicKind::String => {
            let descriptor = dds_dynamic_type_descriptor_t {
                kind: dds_dynamic_type_kind::DDS_DYNAMIC_STRING8,
                name: std::ptr::null(),
                base_type: std::mem::zeroed(),
                discriminator_type: std::mem::zeroed(),
                num_bounds: 0,
                bounds: std::ptr::null(),
                element_type: std::mem::zeroed(),
                key_element_type: std::mem::zeroed(),
            };
            let str_type = dds_dynamic_type_create(participant_entity, descriptor);
            if str_type.ret != 0 {
                return Err(DDSError::from(str_type.ret));
            }
            Ok(dds_dynamic_type_spec_t {
                kind: dds_dynamic_type_spec_kind::DDS_DYNAMIC_TYPE_KIND_DEFINITION,
                type_: dds_dynamic_type_spec__bindgen_ty_1 { type_: str_type },
            })
        }
        _ => Ok(dds_dynamic_type_spec_t {
            kind: dds_dynamic_type_spec_kind::DDS_DYNAMIC_TYPE_KIND_PRIMITIVE,
            type_: dds_dynamic_type_spec__bindgen_ty_1 {
                primitive: kind.dds_kind(),
            },
        }),
    }
}

impl DynamicTypeBuilder {
    pub fn new_struct(type_name: &str) -> Self {
        Self {
            type_name: type_name.to_string(),
            members: Vec::new(),
        }
    }

    /// Add a member. Members are always added with the fixed member index (matching
    /// AUTOID_SEQUENTIAL) and get sequential member ids assigned in declaration order,
    /// starting at 0.
    pub fn member(mut self, name: &str, kind: DynamicKind) -> Self {
        self.members.push((name.to_string(), kind, false));
        self
    }

    /// Mark a previously-added member (by name) as a key field.
    pub fn key(mut self, name: &str) -> Self {
        if let Some(m) = self.members.iter_mut().find(|(n, _, _)| n == name) {
            m.2 = true;
        }
        self
    }

    pub fn build(self, participant: &DdsParticipant) -> Result<DdsDynamicType, DDSError> {
        if self.members.is_empty() {
            return Err(DDSError::BadParameter);
        }

        let participant_entity = unsafe { participant.entity().entity() };
        let type_name_c = CString::new(self.type_name.as_str()).map_err(|_| DDSError::BadParameter)?;

        // Keep every member name's CString alive across the whole build: add_member only
        // reads through the pointer during the call, but that's still for the duration of
        // this function, and it's simplest to just hold them all until we're done.
        let member_names_c: Vec<CString> = self
            .members
            .iter()
            .map(|(name, _, _)| CString::new(name.as_str()))
            .collect::<Result<_, _>>()
            .map_err(|_| DDSError::BadParameter)?;

        unsafe {
            let descriptor = dds_dynamic_type_descriptor_t {
                kind: dds_dynamic_type_kind::DDS_DYNAMIC_STRUCTURE,
                name: type_name_c.as_ptr(),
                base_type: std::mem::zeroed(),
                discriminator_type: std::mem::zeroed(),
                num_bounds: 0,
                bounds: std::ptr::null(),
                element_type: std::mem::zeroed(),
                key_element_type: std::mem::zeroed(),
            };
            let mut dtype = dds_dynamic_type_create(participant_entity, descriptor);
            if dtype.ret != 0 {
                return Err(DDSError::from(dtype.ret));
            }

            let ret = dds_dynamic_type_set_extensibility(
                &mut dtype,
                dds_dynamic_type_extensibility::DDS_DYNAMIC_TYPE_EXT_FINAL,
            );
            if ret != 0 {
                dds_dynamic_type_unref(&mut dtype);
                return Err(DDSError::from(ret));
            }

            for (id, ((_, kind, _), name_c)) in
                self.members.iter().zip(member_names_c.iter()).enumerate()
            {
                let type_spec = match build_member_type_spec(participant_entity, *kind) {
                    Ok(spec) => spec,
                    Err(e) => {
                        dds_dynamic_type_unref(&mut dtype);
                        return Err(e);
                    }
                };
                let member_descriptor = dds_dynamic_member_descriptor_t {
                    name: name_c.as_ptr(),
                    id: id as u32,
                    type_: type_spec,
                    default_value: std::ptr::null_mut(),
                    index: u32::MAX, // DDS_DYNAMIC_MEMBER_INDEX_END: append
                    num_labels: 0,
                    labels: std::ptr::null_mut(),
                    default_label: false,
                };
                let ret = dds_dynamic_type_add_member(&mut dtype, member_descriptor);
                if ret != 0 {
                    dds_dynamic_type_unref(&mut dtype);
                    return Err(DDSError::from(ret));
                }
            }

            for (id, (_, _, is_key)) in self.members.iter().enumerate() {
                if *is_key {
                    let ret = dds_dynamic_member_set_key(&mut dtype, id as u32, true);
                    if ret != 0 {
                        dds_dynamic_type_unref(&mut dtype);
                        return Err(DDSError::from(ret));
                    }
                }
            }

            let mut type_info: *mut ddsi_typeinfo = std::ptr::null_mut();
            let ret = dds_dynamic_type_register(&mut dtype, &mut type_info);
            if ret != 0 {
                dds_dynamic_type_unref(&mut dtype);
                return Err(DDSError::from(ret));
            }
            // dtype is now RESOLVED; both it and type_info are retained in DynamicLayout
            // rather than released here - see the field comment on DynamicLayout for why.

            let mut descriptor: *mut dds_topic_descriptor_t = std::ptr::null_mut();
            let ret = dds_create_topic_descriptor(
                dds_find_scope::DDS_FIND_SCOPE_LOCAL_DOMAIN,
                participant_entity,
                type_info,
                0,
                &mut descriptor,
            );
            if ret != 0 {
                dds_free_typeinfo(type_info);
                dds_dynamic_type_unref(&mut dtype);
                return Err(DDSError::from(ret));
            }

            let m_ops = std::slice::from_raw_parts((*descriptor).m_ops, (*descriptor).m_nops as usize);
            let parse_result = layout::parse(m_ops, &self.members);
            let (size, align) = ((*descriptor).m_size, (*descriptor).m_align);

            // Note: neither type_info nor dtype's reference is released here. Empirically,
            // unref'ing dtype right after this point makes a *second* later
            // dds_create_topic_descriptor call (from DdsDynamicTopic::create(), reusing
            // type_info to back another topic) fail with PRECONDITION_NOT_MET - i.e. the
            // type library does *not* keep a registered type resolvable on its own once its
            // creator's last reference is dropped, despite dds_dynamic_type_register's doc
            // comment implying otherwise ("stored in the type library"). So dtype's
            // reference is kept alive in DynamicLayout for as long as the type might still
            // be needed, and only released in DynamicLayout::drop().
            dds_delete_topic_descriptor(descriptor);

            let fields = match parse_result {
                Ok(fields) => fields,
                Err(e) => {
                    dds_free_typeinfo(type_info);
                    dds_dynamic_type_unref(&mut dtype);
                    return Err(e);
                }
            };
            let field_index = fields
                .iter()
                .enumerate()
                .map(|(i, f)| (f.name.clone(), i))
                .collect();

            Ok(DdsDynamicType(Arc::new(DynamicLayout {
                type_name: self.type_name,
                size,
                align,
                fields,
                field_index,
                dtype,
                type_info,
            })))
        }
    }
}

/// One field's value, kind-checked against a DdsDynamicType's layout by
/// DynamicSample::get()/set(). MVP subset only - see the module doc.
#[derive(Debug, Clone, PartialEq)]
pub enum DynamicValue {
    Bool(bool),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Char8(u8),
    String(String),
}

impl DynamicValue {
    fn kind(&self) -> DynamicKind {
        match self {
            DynamicValue::Bool(_) => DynamicKind::Bool,
            DynamicValue::I8(_) => DynamicKind::I8,
            DynamicValue::U8(_) => DynamicKind::U8,
            DynamicValue::I16(_) => DynamicKind::I16,
            DynamicValue::U16(_) => DynamicKind::U16,
            DynamicValue::I32(_) => DynamicKind::I32,
            DynamicValue::U32(_) => DynamicKind::U32,
            DynamicValue::I64(_) => DynamicKind::I64,
            DynamicValue::U64(_) => DynamicKind::U64,
            DynamicValue::F32(_) => DynamicKind::F32,
            DynamicValue::F64(_) => DynamicKind::F64,
            DynamicValue::Char8(_) => DynamicKind::Char8,
            DynamicValue::String(_) => DynamicKind::String,
        }
    }
}

/// A single sample's worth of raw, C-struct-layout memory for a DdsDynamicType: `size` bytes
/// allocated at `align`, fields accessed by name through the type's parsed layout.
///
/// Every DynamicSample owns its buffer and any indirections it holds (currently just string
/// fields), whether it was built locally via `new()`/`set()` for writing, or deep-copied out
/// of a sample CycloneDDS returned from a take()/read() (`copy_from_raw`) - see that
/// function's doc comment for why it's a copy rather than a borrow of cyclone's own memory.
/// Drop frees the buffer and every owned string.
pub struct DynamicSample {
    dtype: DdsDynamicType,
    buf: std::ptr::NonNull<u8>,
}

unsafe impl Send for DynamicSample {}

impl DynamicSample {
    pub fn new(dtype: &DdsDynamicType) -> Self {
        let alloc_layout = dtype.alloc_layout();
        let buf = unsafe { std::alloc::alloc_zeroed(alloc_layout) };
        let buf = std::ptr::NonNull::new(buf).expect("DynamicSample allocation failed");
        Self {
            dtype: dtype.clone(),
            buf,
        }
    }

    pub fn set(&mut self, name: &str, value: DynamicValue) -> Result<(), DDSError> {
        let field = self.dtype.field(name).ok_or(DDSError::BadParameter)?;
        if field.kind != value.kind() {
            return Err(DDSError::BadParameter);
        }
        let offset = field.offset as usize;
        unsafe {
            let p = self.buf.as_ptr().add(offset);
            match value {
                DynamicValue::Bool(v) => *(p as *mut bool) = v,
                DynamicValue::I8(v) => *(p as *mut i8) = v,
                DynamicValue::U8(v) => *(p as *mut u8) = v,
                DynamicValue::I16(v) => *(p as *mut i16) = v,
                DynamicValue::U16(v) => *(p as *mut u16) = v,
                DynamicValue::I32(v) => *(p as *mut i32) = v,
                DynamicValue::U32(v) => *(p as *mut u32) = v,
                DynamicValue::I64(v) => *(p as *mut i64) = v,
                DynamicValue::U64(v) => *(p as *mut u64) = v,
                DynamicValue::F32(v) => *(p as *mut f32) = v,
                DynamicValue::F64(v) => *(p as *mut f64) = v,
                DynamicValue::Char8(v) => *(p as *mut u8) = v,
                DynamicValue::String(s) => {
                    let ptr_slot = p as *mut *mut std::os::raw::c_char;
                    // Free whatever was there before so overwriting a string field doesn't
                    // leak the previous allocation.
                    let old = *ptr_slot;
                    if !old.is_null() {
                        dds_free(old as *mut std::ffi::c_void);
                    }
                    *ptr_slot = alloc_dds_string(s.as_bytes())?;
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<DynamicValue, DDSError> {
        let field = self.dtype.field(name).ok_or(DDSError::BadParameter)?;
        let offset = field.offset as usize;
        unsafe {
            let p = self.buf.as_ptr().add(offset);
            Ok(match field.kind {
                DynamicKind::Bool => DynamicValue::Bool(*(p as *const bool)),
                DynamicKind::I8 => DynamicValue::I8(*(p as *const i8)),
                DynamicKind::U8 => DynamicValue::U8(*(p as *const u8)),
                DynamicKind::I16 => DynamicValue::I16(*(p as *const i16)),
                DynamicKind::U16 => DynamicValue::U16(*(p as *const u16)),
                DynamicKind::I32 => DynamicValue::I32(*(p as *const i32)),
                DynamicKind::U32 => DynamicValue::U32(*(p as *const u32)),
                DynamicKind::I64 => DynamicValue::I64(*(p as *const i64)),
                DynamicKind::U64 => DynamicValue::U64(*(p as *const u64)),
                DynamicKind::F32 => DynamicValue::F32(*(p as *const f32)),
                DynamicKind::F64 => DynamicValue::F64(*(p as *const f64)),
                DynamicKind::Char8 => DynamicValue::Char8(*(p as *const u8)),
                DynamicKind::String => {
                    let ptr = *(p as *const *const std::os::raw::c_char);
                    if ptr.is_null() {
                        DynamicValue::String(String::new())
                    } else {
                        DynamicValue::String(
                            std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned(),
                        )
                    }
                }
            })
        }
    }

    pub(crate) fn as_ptr(&self) -> *const std::ffi::c_void {
        self.buf.as_ptr() as *const _
    }

    /// Deep-copies a sample CycloneDDS returned from dds_take/dds_read - memory owned by
    /// cyclone's internal loan machinery, valid only until a later take()/read() call
    /// implicitly returns it (see dds_public_loan_api.h) - into a freshly, independently
    /// allocated and owned DynamicSample, including its own copy of any string fields. This
    /// costs one extra allocation+copy per string field versus a borrowing "view" type would,
    /// in exchange for every DynamicSample (whether built locally or read back) having the
    /// same simple, unconditional ownership story.
    ///
    /// # Safety
    /// `src` must point to at least `dtype`'s `size` bytes, laid out per `dtype`'s layout -
    /// i.e. it must actually be a sample of this exact dynamic type, such as one just
    /// returned by dds_take/dds_read on a reader created from this type's topic.
    pub(crate) unsafe fn copy_from_raw(dtype: DdsDynamicType, src: *const std::ffi::c_void) -> Self {
        let sample = Self::new(&dtype);
        std::ptr::copy_nonoverlapping(src as *const u8, sample.buf.as_ptr(), dtype.0.size as usize);

        for field in &dtype.0.fields {
            if field.kind == DynamicKind::String {
                let p = sample.buf.as_ptr().add(field.offset as usize)
                    as *mut *mut std::os::raw::c_char;
                let src_ptr = *p;
                if !src_ptr.is_null() {
                    let bytes = std::ffi::CStr::from_ptr(src_ptr).to_bytes();
                    // alloc_dds_string always succeeds here in practice (a fresh, small
                    // allocation) - copy_from_raw only ever runs synchronously right after a
                    // successful dds_take/dds_read, with no reasonable way to propagate a
                    // Result through that call site without upending DdsDynamicReader::take's
                    // signature for what should never actually fail.
                    *p = alloc_dds_string(bytes).expect("string field re-allocation");
                }
            }
        }
        sample
    }
}

/// Allocates a CycloneDDS-owned (dds_string_alloc'd) copy of `bytes` as a null-terminated C
/// string. dds_string_alloc's `size` parameter is a *character* count - it adds the null
/// terminator's byte itself (see dds_alloc.c: `dds_alloc(size + 1)`) - so this passes
/// `bytes.len()`, not `bytes.len() + 1`.
unsafe fn alloc_dds_string(bytes: &[u8]) -> Result<*mut std::os::raw::c_char, DDSError> {
    let new_ptr = dds_string_alloc(bytes.len()) as *mut std::os::raw::c_char;
    if new_ptr.is_null() {
        return Err(DDSError::OutOfResources);
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), new_ptr as *mut u8, bytes.len());
    *(new_ptr.add(bytes.len())) = 0; // null terminator
    Ok(new_ptr)
}

impl Drop for DynamicSample {
    fn drop(&mut self) {
        unsafe {
            for field in &self.dtype.0.fields {
                if field.kind == DynamicKind::String {
                    let p = self.buf.as_ptr().add(field.offset as usize)
                        as *mut *mut std::os::raw::c_char;
                    let s = *p;
                    if !s.is_null() {
                        dds_free(s as *mut std::ffi::c_void);
                    }
                }
            }
            std::alloc::dealloc(self.buf.as_ptr(), self.dtype.alloc_layout());
        }
    }
}

/// A topic backed by a dynamic type. Unlike `DdsTopic<T>`, this doesn't go through
/// SerType<T>/dds_create_topic_sertype - it's a `dds_create_topic` call against a
/// dds_topic_descriptor_t CycloneDDS re-derives (a second time - the first was inside
/// DynamicTypeBuilder::build(), purely to parse the layout) from the type's retained
/// type_info, matching the C API's own documented lifecycle for this call.
pub struct DdsDynamicTopic {
    entity: DdsEntity,
    dtype: DdsDynamicType,
    _listener: Option<DdsListener>,
}

impl DdsDynamicTopic {
    pub fn create(
        participant: &DdsParticipant,
        dtype: &DdsDynamicType,
        topic_name: &str,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        let participant_entity = unsafe { participant.entity().entity() };
        let name_c = CString::new(topic_name).map_err(|_| DDSError::BadParameter)?;

        unsafe {
            let mut descriptor: *mut dds_topic_descriptor_t = std::ptr::null_mut();
            let ret = dds_create_topic_descriptor(
                dds_find_scope::DDS_FIND_SCOPE_LOCAL_DOMAIN,
                participant_entity,
                dtype.0.type_info,
                0,
                &mut descriptor,
            );
            if ret != 0 {
                return Err(DDSError::from(ret));
            }

            let topic = dds_create_topic(
                participant_entity,
                descriptor,
                name_c.as_ptr(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );
            dds_delete_topic_descriptor(descriptor);

            if topic >= 0 {
                Ok(DdsDynamicTopic {
                    entity: DdsEntity::new(topic),
                    dtype: dtype.clone(),
                    _listener: maybe_listener,
                })
            } else {
                Err(DDSError::from(topic))
            }
        }
    }

    pub fn dynamic_type(&self) -> &DdsDynamicType {
        &self.dtype
    }
}

impl crate::Entity for DdsDynamicTopic {
    fn entity(&self) -> &DdsEntity {
        &self.entity
    }
}

pub struct DdsDynamicWriter {
    entity: DdsEntity,
    dtype: DdsDynamicType,
    _listener: Option<DdsListener>,
}

impl DdsDynamicWriter {
    pub fn create(
        entity: &dyn crate::DdsWritable,
        topic: &DdsDynamicTopic,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_writer(
                entity.entity().entity(),
                topic.entity.entity(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );
            if w >= 0 {
                Ok(DdsDynamicWriter {
                    entity: DdsEntity::new(w),
                    dtype: topic.dtype.clone(),
                    _listener: maybe_listener,
                })
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn dynamic_type(&self) -> &DdsDynamicType {
        &self.dtype
    }

    /// Allocate a new, zeroed DynamicSample matching this writer's type, ready to fill via
    /// `set()` and publish via `write()`.
    pub fn new_sample(&self) -> DynamicSample {
        DynamicSample::new(&self.dtype)
    }

    pub fn write(&mut self, sample: &DynamicSample) -> Result<(), DDSError> {
        let ret = unsafe { dds_write(self.entity.entity(), sample.as_ptr()) };
        if ret >= 0 {
            Ok(())
        } else {
            Err(DDSError::from(ret))
        }
    }
}

impl crate::Entity for DdsDynamicWriter {
    fn entity(&self) -> &DdsEntity {
        &self.entity
    }
}

pub struct DdsDynamicReader {
    entity: DdsEntity,
    dtype: DdsDynamicType,
    _listener: Option<DdsListener>,
}

impl DdsDynamicReader {
    pub fn create(
        entity: &dyn crate::DdsReadable,
        topic: &DdsDynamicTopic,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let r = dds_create_reader(
                entity.entity().entity(),
                topic.entity.entity(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );
            if r >= 0 {
                Ok(DdsDynamicReader {
                    entity: DdsEntity::new(r),
                    dtype: topic.dtype.clone(),
                    _listener: maybe_listener,
                })
            } else {
                Err(DDSError::from(r))
            }
        }
    }

    pub fn dynamic_type(&self) -> &DdsDynamicType {
        &self.dtype
    }

    /// Synchronously take up to `max` samples. Returns owned DynamicSamples (deep-copied out
    /// of cyclone's loaned memory - see DynamicSample::copy_from_raw), so unlike
    /// SampleBuffer<T> there's no separate "is this slot valid" check the caller needs to
    /// make afterward: invalid_data slots are filtered out here already.
    pub fn take(&self, max: usize) -> Result<Vec<DynamicSample>, DDSError> {
        let mut ptrs: Vec<*mut std::ffi::c_void> = vec![std::ptr::null_mut(); max];
        let mut infos: Vec<dds_sample_info_t> = vec![unsafe { std::mem::zeroed() }; max];

        unsafe {
            let ret = dds_take(
                self.entity.entity(),
                ptrs.as_mut_ptr(),
                infos.as_mut_ptr(),
                max,
                max as u32,
            );
            if ret < 0 {
                return Err(DDSError::from(ret));
            }
            let mut out = Vec::with_capacity(ret as usize);
            for i in 0..ret as usize {
                if infos[i].valid_data {
                    out.push(DynamicSample::copy_from_raw(self.dtype.clone(), ptrs[i]));
                }
            }
            Ok(out)
        }
    }
}

impl crate::Entity for DdsDynamicReader {
    fn entity(&self) -> &DdsEntity {
        &self.entity
    }
}
