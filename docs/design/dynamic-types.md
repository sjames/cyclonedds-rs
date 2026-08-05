# Dynamic Types: Analysis and Design

Status: **Phase 1 (MVP) implemented** - see `src/dds_dynamic_type.rs` and
`tests/dynamic_type_test.rs`. Phases 2-4 (§5) remain design only.

One correction to the design below, discovered during implementation: `dds_dynamic_type_t`'s
own reference must be kept alive for as long as its `type_info` might still be used to derive
another `dds_topic_descriptor_t` (e.g. for a second topic off the same type) - unref'ing it
right after `dds_dynamic_type_register()` makes a later `dds_create_topic_descriptor` call
using that `type_info` fail with `PRECONDITION_NOT_MET`, despite `register()`'s own doc
comment implying the type library keeps a registered type resolvable on its own. `DdsDynamicType`
now retains both.

## 1. Summary

CycloneDDS 11's Dynamic Type API (`dds_public_dynamic_type.h`) lets an application
construct a topic type's structure at runtime — members, keys, extensibility — instead of
generating a Rust struct at compile time via `cdds_derive`. This document works out what it
would actually take to expose that conveniently from `cyclonedds-rs`.

The headline finding: **dynamic types don't go through this crate's existing serialization
machinery at all.** They ride on CycloneDDS's own built-in default sertype (the same one
`idlc`-generated C structs use), driven by a `dds_topic_descriptor_t` that CycloneDDS
computes for us. That's good news for the type-definition half of this feature (no need to
extend `serdes.rs`'s `ddsi_sertype_ops`/`ddsi_serdata_ops` plugin at all) and the hard part
for the sample-data half (samples are raw, C-struct-layout memory blobs — there is no
`Sample<T>`/`Arc<T>`/serde story available to lean on).

Recommendation: **worth doing, but only for a deliberately narrow first slice** — flat
structs, `FINAL` extensibility, primitive + string members, no nesting/unions/sequences/
arrays. That slice is genuinely tractable and already covers a real use case (a generic
bridge/inspector that doesn't know types at compile time). Extending it to nested types,
collections, and unions is a much larger, separate effort that should be scoped later, once
the first slice has proven the approach against a live participant.

## 2. What the C API actually gives you

### 2.1 Building a type

`dds_dynamic_type_create(entity, descriptor) -> dds_dynamic_type_t` starts construction.
The type is a mutable "under construction" handle until registered. Members/properties are
added with a family of calls, all taking `&mut dds_dynamic_type_t` and returning a
`dds_return_t` (the type also latches the first error into its own `.ret` field):

- `dds_dynamic_type_add_member(type, member_descriptor)` — add a struct/union field. The
  descriptor carries name, id (or `AUTO`), type (primitive kind or a reference to another
  `dds_dynamic_type_t`), and (for unions) case labels. **Correction from implementation**:
  despite `DDS_DYNAMIC_STRING8`/`STRING16` being listed among the primitive
  `dds_dynamic_type_kind_t` values, `add_member` rejects them as an inline primitive spec
  (`BAD_PARAMETER`) - a string member needs its own standalone `dds_dynamic_type_t` created
  first (`dds_dynamic_type_create` with `kind: DDS_DYNAMIC_STRING8`) and referenced the same
  way a nested struct would be. True scalars (bool, integers, floats, char8) work inline as
  designed below.
- `dds_dynamic_type_set_extensibility(type, FINAL | APPENDABLE | MUTABLE)`
- `dds_dynamic_type_set_bit_bound`, `set_nested`, `set_autoid` (sequential vs. hashed member
  ids)
- `dds_dynamic_type_add_enum_literal`, `add_bitmask_field` for those two kinds
- `dds_dynamic_member_set_key(type, member_id, bool)`, `set_optional`, `set_external`,
  `set_hashid`, `set_must_understand` — per-member flags, set after the member exists

Composite members (nested struct, array, sequence, map, alias) reference *another*
`dds_dynamic_type_t` via `dds_dynamic_type_spec_t`. Ownership transfers to the parent on
`add_member`; `dds_dynamic_type_ref`/`unref`/`dup` manage sharing a subtype across multiple
parents (e.g. the same nested struct used by two fields).

### 2.2 Registering and creating a topic

```
dds_dynamic_type_register(&mut dtype, &mut type_info)   // dtype -> RESOLVED, immutable
dds_create_topic_descriptor(scope, participant, type_info, timeout, &mut descriptor)
dds_create_topic(participant, descriptor, name, qos, listener)   // the *existing* C entry point
dds_free_typeinfo(type_info)
dds_delete_topic_descriptor(descriptor)
dds_dynamic_type_unref(&mut dtype)
```

This is confirmed against CycloneDDS's own `tests/dynamic_type.c` (`do_test`, line ~38). The
last three calls are cleanup — the topic entity retains whatever it needs internally, so all
of it can be freed immediately after `dds_create_topic` returns, matching the existing
`SerType<T>::new()` builder-then-consume pattern.

The important part: **`dds_create_topic` is the same function `DdsTopic::<T>::create` already
calls.** A dynamic-type topic is a normal topic from CycloneDDS's point of view — it just got
its `dds_topic_descriptor_t` synthesized at runtime instead of generated by `idlc` at build
time.

### 2.3 The `dds_topic_descriptor_t` shape

```c
typedef struct dds_topic_descriptor {
  const uint32_t m_size;      // sizeof the in-memory C struct
  const uint32_t m_align;     // alignof it
  const uint32_t m_flagset;
  const uint32_t m_nkeys;
  const char *m_typename;
  const dds_key_descriptor_t *m_keys;
  const uint32_t m_nops;
  const uint32_t *m_ops;      // the marshalling bytecode - see below
  const char *m_meta;
  ...
} dds_topic_descriptor_t;
```

`m_ops` is CycloneDDS's internal "stream VM" bytecode (`dds_opcodes.h`, already partly bound
in `cyclonedds-sys` as `dds_stream_opcode`/`dds_stream_typecode`) — the same instruction set
`idlc` emits for statically-compiled types. Each `DDS_OP_ADR` instruction (one per member, for
a flat struct) **carries the member's byte offset as an explicit operand.** This is the load-
bearing fact for everything in §3: CycloneDDS is not asking us to reimplement a struct-layout
algorithm and hope it matches — it publishes the offsets it actually computed, in a documented,
public header, specifically so language bindings can build exactly this kind of "read arbitrary
opcodes" support without guessing.

There is **no dynamic-data (sample get/set by name) API in `dds_public_dynamic_type.h` or
anywhere else in `dds/ddsc/include`.** CycloneDDS's own test suite (`dynamic_type.c`) never
writes or reads a real sample for a dynamically-created type — it only exercises type
construction, registration, and topic creation, then compares type ids against a statically
generated reference type. Sample manipulation is squarely the language binding's job, and
that's what most of this document is about.

## 3. The core design problem: reading and writing sample data

Since a dynamic-type topic uses CycloneDDS's default sertype, `dds_write`/`dds_read`/
`dds_take` expect a plain pointer to `m_size` bytes of memory laid out exactly as `m_ops`
describes — the same as if it were a `#[repr(C)]` struct matching an `idlc`-generated one.
There is no `Sample<T>`, no `Arc<T>`, nothing from `serdes.rs` involved: this bypasses our
`ddsi_sertype_ops`/`ddsi_serdata_ops` plugin entirely.

### 3.1 Two ways to get field offsets

**(a) Track our own layout while building the type.** Since our Rust builder is the one
calling `dds_dynamic_type_add_member` for every field, it already knows the ordered list of
`(name, kind)`. In principle we could compute offsets ourselves using ordinary
natural-alignment rules and never look at `m_ops` at all.

**(b) Parse `m_ops` after `dds_create_topic_descriptor` returns.** Walk the opcode array,
pull the offset out of each top-level `ADR` instruction, and zip that 1:1 against our
ordered member list (for a flat, `FINAL` struct the ops appear in declaration order).

**(b) is the only option that's actually safe.** (a) requires our layout algorithm to
provably match CycloneDDS's for every case we support, forever, across CycloneDDS versions —
any divergence is silent memory corruption (we'd write a field at the offset *we* computed,
CycloneDDS would read/serialize it from the offset *it* computed). (b) uses the numbers
CycloneDDS itself produced, so there's no algorithm to keep in sync. The one-time cost is
writing a (small, for the flat/FINAL case) opcode parser in Rust.

Reassuring side note: extensibility (`FINAL`/`APPENDABLE`/`MUTABLE`) is a *wire encoding*
concern — it changes how `m_ops` tells the runtime to serialize the fixed in-memory layout to
CDR, not the in-memory layout itself. So supporting `APPENDABLE`/`MUTABLE` later shouldn't
require touching offset computation, just accepting that member order in `m_ops` may need
more careful handling than "first N `ADR`s in file order" (mutable types use `PLM`/`MID` with
explicit member ids and can reorder).

### 3.2 Members with indirection: strings

A `string` member is stored as a `char *` in the C struct, not inline bytes. Writing one means
heap-allocating via `dds_string_alloc`/`dds_alloc` (already bound) and storing the pointer at
the field's offset; reading one CycloneDDS hands back means eventually freeing it via
`dds_sample_free` with the right `dds_free_op_t` (already bound) — the same convention
`topic_type_methods.rs`/`SampleBuffer<T>` presumably already has to respect for statically
generated string fields, worth checking against before reinventing it.

This is real, separate complexity from fixed-size POD fields — a natural line to draw between
an even-smaller "POD-only" slice and a "+ strings" one.

### 3.3 Members that don't fit the "one buffer, flat offsets" model

Nested structs, arrays, sequences, and unions all need more than "offset + primitive kind":

- **Nested struct**: fine, in principle — its own sub-region of the same buffer, own offset
  table, recursively.
- **Fixed array**: contiguous, offset + stride, fine.
- **Sequence**: stored as a `{ length, capacity, buffer-pointer }` struct in memory (CycloneDDS's
  usual `dds_sequence_t`-style representation) — separate heap allocation, own ownership/free
  story, same shape as the string problem but generalized.
- **Union**: the in-memory layout depends on which case is active (discriminant + one active
  member sharing storage with the others) — offset alone isn't enough, need the discriminant
  value and the case→offset mapping, and get this wrong and you read/write outside the active
  case's actual type.

All tractable by parsing more of the opcode set (`DDS_OP_SEQ`, `DDS_OP_ARR`, `DDS_OP_UNI`/
`JEQ4`, `DDS_OP_JSR` for recursion into nested types), but each is its own chunk of work and
its own way to get subtly wrong. This is why §5 phases them out of the first slice.

## 4. Proposed Rust API shape

Everything here is a *sketch* to validate the shape of the problem, not a spec to implement
verbatim.

### 4.1 Type construction — a builder, hiding the C descriptor structs

```rust
let dtype = DynamicTypeBuilder::new_struct("MyApp::Sensor")
    .member("id", DynamicKind::UInt32)
    .key("id")                       // marks the member added most recently, or by name
    .member("temperature", DynamicKind::Float64)
    .member("label", DynamicKind::String)
    .extensibility(Extensibility::Final)
    .build(&participant)?;           // -> DdsDynamicType (RESOLVED, owns a live m_ops/m_size)
```

`DynamicKind` is a thin Rust enum mirroring `dds_dynamic_type_kind_t`'s primitives (plus,
later, `Struct(DdsDynamicType)`, `Array{..}`, `Sequence{..}`, once §3.3 is tackled). `build()`
internally does create → add_member* → set flags → register → create_topic_descriptor,
capturing `m_size`/`m_align` and the parsed offset table (§3.1b), then frees the transient
`type_info`/`descriptor` per §2.2 — the caller never sees a raw `dds_dynamic_type_t` or
`dds_topic_descriptor_t`.

### 4.2 Topic/writer/reader — necessarily a parallel, non-generic family

`DdsTopic<T>`/`DdsWriter<T>`/`DdsReader<T>` are all bound on `T: TopicType`, and `TopicType`
is precisely the trait that plugs a type into *this crate's* serdes machinery (`Serialize` +
`DeserializeOwned` + `key_cdr()` + ...). A dynamic type has no such Rust type at compile time
and doesn't go through that machinery at all (§2.2, §3) — so it cannot reuse `DdsWriter<T>` by
finding some clever marker `T`. It needs its own non-generic types:

```rust
pub struct DdsDynamicTopic { .. }     // wraps the dds_entity_t + the DdsDynamicType's layout
pub struct DdsDynamicWriter { .. }    // dds_create_writer + raw dds_write(entity, buf.as_ptr())
pub struct DdsDynamicReader { .. }    // dds_create_reader + raw dds_take/dds_read
```

### 4.3 Sample data — a value map over the parsed layout

```rust
let mut sample = topic.new_sample();               // zeroed m_size-byte buffer + layout ref
sample.set("id", DynamicValue::UInt32(7))?;
sample.set("temperature", DynamicValue::Float64(36.6))?;
sample.set("label", DynamicValue::String("probe-7".into()))?;
writer.write(&sample)?;                             // dds_write(entity, sample.as_ptr())

let mut buf = reader.take(32)?;
for sample in buf.valid_samples() {
    let id = sample.get("id")?.as_u32()?;
    ...
}
```

`DynamicValue` is the obvious `enum { Bool, I8, U8, .., F64, String(String), .. }` (extended to
`Struct(DynamicSample)`, `Array(Vec<DynamicValue>)`, etc. once §3.3 lands). `set`/`get` do
bounds/kind-checked raw pointer read-write at the offset the layout table has for that name,
returning a typed error rather than panicking on a name/kind mismatch.

No existing crate solves the actual hard part here (mapping onto CycloneDDS's own raw `m_ops`
byte offsets) — that's inherently bespoke regardless of what value type sits on top. For the
*shape* of `DynamicValue` itself, worth knowing about but not depending on: `apache-avro`'s
`Value` enum has almost the same variant set as XTypes' primitive/aggregate kinds (not worth
pulling in the whole Avro schema/encoding stack just for one enum); `serde_value` is a smaller,
schema-less "detached value" type closer to what the *extracted, owned* half of the problem
needs; Cap'n Proto's `capnp::dynamic_value`/`DynamicStruct`/`StructSchema` API (schema object +
dynamic struct backed by raw message memory with computed offsets + typed accessors) is the
closest architectural precedent for the whole mechanism, worth reading before finalizing this
API even though none of its code applies (different wire format). Recommend writing our own
small enum rather than depending on any of these.

This is the natural place for a `serde_json::Value` convenience layer later (`sample.to_json()`
/ `DynamicSample::from_json(&layout, &json)`) if a genuinely ergonomic "construct from/inspect
as JSON" story is wanted — that's the one case a dependency might genuinely pay off, since it's
the "generic value" format most users would actually want to bridge to/from. Still sugar on top
of `DynamicValue`, not a prerequisite.

`Drop for DynamicSample` needs to free any owned indirections (strings, later sequences) it
still holds via `dds_sample_free`, mirroring `SampleBuffer<T>`'s existing cleanup
responsibilities for the static-type path.

## 5. Proposed phasing

1. **MVP — flat, `FINAL`, POD-only.** Primitives only, no strings, no nesting. Proves the
   whole pipeline end to end (builder → register → topic descriptor → opcode-offset parsing →
   raw write/read → value round-trip) against a real participant, with the smallest possible
   surface for something to be wrong in. This is the phase to actually build and test before
   committing to any of the API shape in §4.
2. **+ Strings.** Adds the heap-allocation/`dds_sample_free` ownership story (§3.2). Now
   covers a realistic "generic sensor/telemetry struct" use case.
3. **+ Nested structs, fixed arrays.** Extends the opcode parser to recurse via `DDS_OP_JSR`
   and to handle contiguous `DDS_OP_ARR` regions. Still no variable-length collections or
   discriminated unions.
4. **+ Sequences, unions, `APPENDABLE`/`MUTABLE`.** The general case. Meaningfully more opcode
   surface (`DDS_OP_SEQ`, `DDS_OP_UNI`/`JEQ4`, `PLM`/`MID` for mutable member reordering).
   Worth scoping as its own effort once 1-3 exist and have a real user.

Phase 1 alone is enough to validate whether the API shape in §4 actually feels good to use;
it's the right place to stop and reassess before sinking time into 3-4.

## 6. What's needed in `cyclonedds-sys`

None of this is bound yet. New `allowlist_function`/`allowlist_type` entries needed for:

- `dds_dynamic_type_create`, `_register`, `_ref`, `_unref`, `_dup`
- `dds_dynamic_type_set_extensibility`, `_set_bit_bound`, `_set_nested`, `_set_autoid`
- `dds_dynamic_type_add_member`, `_add_enum_literal`, `_add_bitmask_field`
- `dds_dynamic_member_set_key`, `_set_optional`, `_set_external`, `_set_hashid`,
  `_set_must_understand`
- `dds_create_topic_descriptor`, `dds_delete_topic_descriptor`, `dds_free_typeinfo`
- Types: `dds_dynamic_type_t`, `dds_dynamic_type_descriptor_t`,
  `dds_dynamic_member_descriptor_t`, `dds_dynamic_type_spec_t`, `dds_dynamic_type_kind_t`
  (rustified enum, matching the `dds_qos_kind` fix from the QoS Provider work),
  `dds_find_scope_t` (rustified enum), `dds_topic_descriptor_t`

All gated behind `DDS_HAS_TYPELIB` (dynamic type API) and `DDS_HAS_TYPE_DISCOVERY` (topic
descriptor lookup) on the C side. Confirmed both are already on in the vendored build:
`ENABLE_TYPELIB` defaults to `ON` in CycloneDDS's own `CMakeLists.txt` and `ENABLE_TYPE_DISCOVERY`
(which `build.rs` already passes explicitly) hard-requires it, so no new CMake flags are
needed in `build.rs` for this.

Several of the descriptor structs (`dds_dynamic_type_descriptor_t`,
`dds_dynamic_member_descriptor_t`) contain nested unions/arrays-of-pointers
(`dds_dynamic_type_spec_t`, `labels: *mut i32`) — bindgen should handle them fine, but the
convenience macros in the header (`DDS_DYNAMIC_MEMBER`, `DDS_DYNAMIC_TYPE_SPEC`, etc.) are
C preprocessor macros bindgen won't translate; the Rust builder in §4.1 needs to construct
those descriptor structs by hand, which is exactly the kind of detail the builder should
hide from callers.

## 7. Open questions / risks

- **Opcode parser correctness is the whole ballgame.** Every phase's data-access story
  depends on correctly walking `m_ops`. Getting an offset wrong doesn't fail loudly — it
  reads/writes the wrong bytes. Needs deliberate, direct tests (write via `DynamicSample`,
  read back via a *statically*-defined `#[derive(Topic)]` Rust struct with the matching
  layout, or vice versa) to cross-check against the existing, trusted static path rather than
  only testing dynamic-against-dynamic.
- **Versioning risk.** `m_ops`'s bytecode format is CycloneDDS-internal (if public/stable
  enough for bindings to rely on, per `dds_opcodes.h` being a public header — but "public
  header" isn't the same guarantee as "stable ABI across major versions"). Worth checking
  upstream's own stability stance on this format before leaning on it long-term.
- **Where do dynamically-typed samples fit relative to `SampleBuffer<T>`/`TopicType`?** §4.2
  proposes a fully parallel type family rather than trying to unify with the generic
  `DdsWriter<T>`/`DdsReader<T>` API. That's simpler to build but means two separate APIs
  users need to learn; worth revisiting once Phase 1 exists, in case a shared trait
  (`DdsWritable`/`DdsReadable`-style) can at least unify entity-management code even if the
  sample representation stays genuinely different.
- **Value proposition vs. cost, revisited.** This is a meaningfully bigger effort than any
  other feature done so far this session, even scoped to Phase 1. It's the right feature for
  someone building a generic bridge/inspector/gateway tool; it is not needed for typed pub/sub
  application code, which is what `cdds_derive` already serves well. Worth confirming that's
  actually the use case driving interest before investing past Phase 1.
