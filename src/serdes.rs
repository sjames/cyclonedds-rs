/*
    Copyright 2021 Sojan James

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

// Rust deserializer for CycloneDDS.
// See discussion at https://github.com/eclipse-cyclonedds/cyclonedds/issues/830

use cdr::{Bounded, CdrBe, Infinite};


use serde::{de::DeserializeOwned, Serialize};
use std::io::prelude::*;

use std::ptr::NonNull;

use std::{
    ffi::{c_void, CStr},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use cyclonedds_sys::*;
//use fasthash::{murmur3::Hasher32, FastHasher};
use murmur3::murmur3_32;
use std::io::Cursor;

// CycloneDDS's from_sample callback is invoked with two structurally different pointers,
// with no way to tell which one it got:
//
//  - DdsWriter::write() wraps the application's data in a Sample<T> before calling
//    dds_write(), so cyclone hands the *same* Sample<T> pointer back to from_sample. This is
//    what lets a local, in-process reader share the writer's Arc<T> with zero copying.
//  - DdsWriter::loan()/return_loan() hand cyclone a raw `*mut T` (from dds_request_loan) that
//    the application fills in directly. Cyclone's dds_write_impl_psmxloan_serdata sometimes
//    calls from_sample with that exact raw pointer too - specifically to build a "regular"
//    (non-PSMX) serdata for consumers not on the zero-copy SHM path. The genuine SHM
//    zero-copy delivery to PSMX-matched readers never goes through from_sample at all, so
//    this branch being non-zero-copy costs nothing we actually wanted to keep.
//
// This registry closes that gap: DdsWriter::loan() marks the address it hands out, Loaned<T>
// (on drop, which happens after the matching dds_write() call inside return_loan() has
// already run) unmarks it, and from_sample checks membership to decide which shape it got.
mod loan_registry {
    use std::any::TypeId;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Mutex, OnceLock};

    fn registry() -> &'static Mutex<HashMap<TypeId, HashSet<usize>>> {
        static REGISTRY: OnceLock<Mutex<HashMap<TypeId, HashSet<usize>>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(crate) fn mark_loaned<T: 'static>(addr: usize) {
        registry()
            .lock()
            .unwrap()
            .entry(TypeId::of::<T>())
            .or_default()
            .insert(addr);
    }

    pub(crate) fn unmark_loaned<T: 'static>(addr: usize) {
        if let Some(set) = registry().lock().unwrap().get_mut(&TypeId::of::<T>()) {
            set.remove(&addr);
        }
    }

    pub(crate) fn is_loaned<T: 'static>(addr: usize) -> bool {
        registry()
            .lock()
            .unwrap()
            .get(&TypeId::of::<T>())
            .is_some_and(|set| set.contains(&addr))
    }
}
pub(crate) use loan_registry::{is_loaned, mark_loaned, unmark_loaned};

// A second, independent registry for DdsWriter::loan_of_size()/loan_serialized() loans.
// Those go through the *same* from_loaned_sample callback as the fixed-size loans above, but
// the buffer holds pre-serialized CDR bytes rather than a raw T - cyclone's own loan metadata
// (sample_state, sample_size) doesn't reliably distinguish the two cases (both read back as
// DDS_LOANED_SAMPLE_STATE_RAW_DATA once dds_write() picks up the loan), so from_loaned_sample
// needs its own way to tell which convention a given address was filled with. Kept separate
// from loan_registry above rather than adding a "kind" to it, to avoid touching that
// already-debugged mechanism.
mod raw_loan_registry {
    use std::any::TypeId;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    // Maps loan address -> requested size. Both from_sample and from_loaned_sample need to
    // recognize a raw-loan address (see their call sites), but from_sample only ever
    // receives the bare pointer with no size hint of its own, so the registry has to carry
    // the size rather than just membership.
    fn registry() -> &'static Mutex<HashMap<TypeId, HashMap<usize, u32>>> {
        static REGISTRY: OnceLock<Mutex<HashMap<TypeId, HashMap<usize, u32>>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(crate) fn mark_raw_loaned<T: 'static>(addr: usize, size: u32) {
        registry()
            .lock()
            .unwrap()
            .entry(TypeId::of::<T>())
            .or_default()
            .insert(addr, size);
    }

    pub(crate) fn unmark_raw_loaned<T: 'static>(addr: usize) {
        if let Some(map) = registry().lock().unwrap().get_mut(&TypeId::of::<T>()) {
            map.remove(&addr);
        }
    }

    pub(crate) fn raw_loaned_size<T: 'static>(addr: usize) -> Option<u32> {
        registry()
            .lock()
            .unwrap()
            .get(&TypeId::of::<T>())
            .and_then(|map| map.get(&addr).copied())
    }
}
pub(crate) use raw_loan_registry::{mark_raw_loaned, raw_loaned_size, unmark_raw_loaned};

#[repr(C)]
pub struct SerType<T> {
    sertype: ddsi_sertype,
    _phantom: PhantomData<T>,
}

pub trait TopicType: Serialize + DeserializeOwned {
    // generate a non-cryptographic hash of the key values to be used internally
    // in cyclonedds
    fn hash(&self, basehash : u32) -> u32 {
        let cdr = self.key_cdr();
        let mut cursor = Cursor::new(cdr.as_slice());
        murmur3_32(&mut cursor, 0).unwrap() ^ basehash
    }

    fn is_fixed_size() -> bool {
        false
    }
    /// The type name for this topic
    fn typename() -> std::ffi::CString {
        let ty_name_parts: String = std::any::type_name::<Self>()
            .split("::")
            .skip(1)
            .collect::<Vec<_>>()
            .join("::");

        
        //println!("Typename:{:?}", &typename);
        std::ffi::CString::new(ty_name_parts).expect("Unable to create CString for type name")
    }

    /// The default topic_name to use when creating a topic of this type. The default
    /// implementation uses '/' instead of '::' to form a unix like path.
    /// A prefix can optionally be added
    fn topic_name(maybe_prefix: Option<&str>) -> String {
        let topic_name_parts: String = format!(
            "/{}",
            std::any::type_name::<Self>()
                .to_string()
                .split("::")
                .skip(1)
                .collect::<Vec<_>>()
                .join("/")
        );

        if let Some(prefix) = maybe_prefix {
            let mut path = String::from(prefix);
            path.push_str(&topic_name_parts);
            path
        } else {
            topic_name_parts
        }
    }

    fn has_key() -> bool;
    // this is the key as defined in the DDS-RTPS spec.
    // KeyHash (PID_KEY_HASH). This function does not
    // hash the key. Use the force_md5_keyhash to know
    // whether to use md5 even if the the key cdr is 16 bytes
    // or shorter.
    fn key_cdr(&self) -> Vec<u8>;

    // force the use of md5 even if the serialized size is less than 16
    // as per the standard, we need to check the potential field size and not the actual.
    fn force_md5_keyhash() -> bool;
}

impl<'a, T> SerType<T> {
    pub fn new() -> Box<SerType<T>>
    where
        T: DeserializeOwned + Serialize + TopicType + 'static,
    {
        Box::<SerType<T>>::new(SerType {
            sertype: {
                let mut sertype = std::mem::MaybeUninit::uninit();
                unsafe {
                    let type_name = T::typename();
                    ddsi_sertype_init(
                        sertype.as_mut_ptr(),
                        type_name.as_ptr(),
                        Box::into_raw(create_sertype_ops::<T>()),
                        Box::into_raw(create_serdata_ops::<T>()),
                        !T::has_key(),
                    );
                    let mut sertype = sertype.assume_init();
                    sertype.set_is_memcpy_safe(if T::is_fixed_size() { 1 } else { 0 });
                    sertype.sizeof_type = std::mem::size_of::<T>() as u32;
                    sertype
                }
            },
            _phantom: PhantomData,
        })
    }

    // cast into cyclone dds sertype.  Rust relinquishes ownership here.
    // Cyclone DDS will free this. But if you need to free this pointer
    // before handing it over to cyclone, make sure you explicitly free it
    pub fn into_sertype(sertype: Box<SerType<T>>) -> *mut ddsi_sertype {
        Box::<SerType<T>>::into_raw(sertype) as *mut ddsi_sertype
    }

    pub fn try_from_sertype(sertype: *const ddsi_sertype) -> Option<Box<SerType<T>>> {
        let ptr = sertype as *mut SerType<T>;
        if !ptr.is_null() {
            Some(unsafe { Box::from_raw(ptr) })
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub enum SampleStorage<T> {
    Owned(Arc<T>),
    Loaned(Arc<NonNull<T>>),
}

impl<T> Deref for SampleStorage<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            SampleStorage::Owned(t) => t.deref(),
            SampleStorage::Loaned(t) => unsafe { t.as_ref().as_ref() },
        }
    }
}

impl<T> Drop for SampleStorage<T> {
    fn drop(&mut self) {
        match self {
            SampleStorage::Loaned(_t) => {
            }
            _ => {

            }
        }
    }
}


pub struct Sample<T> {
    //Serdata is used for incoming samples. We hold a reference to the ddsi_serdata which contains 
    // the sample
    serdata: Option<*mut ddsi_serdata>,
    // sample is used for outgoing samples.
    sample: Option<SampleStorage<T>>,
}

impl<'a,T> Sample<T>
where
    T: TopicType
{
    pub fn try_deref<>(&self) -> Option<&T> {       
            if let Some(serdata) = self.serdata {
                let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
                match &serdata.sample {
                    SampleData::Uninitialized => None,
                    SampleData::SDKKey => None,
                    SampleData::SDKData(it) => Some(it.as_ref()),
                    SampleData::SHMData(it) => unsafe { Some(it.as_ref())},
                }
            } else {
                None
            }
  
    }

    pub fn get_sample(&self) -> Option<SampleStorage<T>> {
        //if let Ok(t) = self.sample.write() {
            match self.sample.as_ref() {
                Some(s) => match s {
                    SampleStorage::Owned(s) => Some(SampleStorage::Owned(s.clone())),
                    SampleStorage::Loaned(s) => Some(SampleStorage::Loaned(s.clone())),
                },
                None => None,
            }
    }

    // Deprecated as this function can panic
    #[deprecated]
    pub (crate)fn get(&self) -> Option<Arc<T>> {
        //let t = self.sample;
        match &self.sample {
            Some(SampleStorage::Owned(t)) => Some(t.clone()),
            Some(SampleStorage::Loaned(_t)) => {
                None
            }
            None => {
                None
            }
        }
    }

    pub(crate) fn set_serdata(&mut self,serdata:*mut ddsi_serdata) {
        // Increment the reference count
        unsafe {ddsi_serdata_addref(serdata);}
        self.serdata = Some(serdata)
    }

    pub fn set(&mut self, t: Arc<T>) {
        //let mut sample = self.sample.write().unwrap();
        self.sample.replace(SampleStorage::Owned(t));
    }

    pub fn set_loaned(&mut self, t: NonNull<T>) {
        //let mut sample = self.sample.write().unwrap();
        self.sample.replace(SampleStorage::Loaned(Arc::new(t)));
    }

    pub fn clear(&mut self) {
        //let mut sample = self.sample.write().unwrap();
        let t = self.sample.take();

        match &t {
            Some(SampleStorage::Owned(_o)) => {}
            Some(SampleStorage::Loaned(_o)) => {}
            None => {}
        }
    }

    pub fn from(it: Arc<T>) -> Self {
        Self {
            serdata : None,
            sample: Some(SampleStorage::Owned(it)),
        }
    }
}

impl<T> Default for Sample<T> {
    fn default() -> Self {
        Self {
            serdata : None,
            sample: None,
        }
    }
}

impl<T> Drop for Sample<T> {
    fn drop(&mut self) {
        if let Some(serdata) = self.serdata {
            unsafe {ddsi_serdata_removeref(serdata)};
        }
    }
}




///
/// TODO: UNSAFE WARNING Review needed. Forcing SampleBuffer<T> to be Send
/// DDS read API uses an array of void* pointers. The SampleBuffer<T> structure
/// is used to create the sample array in the necessary format.
/// We allocate the Sample<T> structure and set it to deallocated here.
/// Cyclone does not allocate the sample, it only sets the value of the Arc<T>
/// inside the Sample<T>::Value<Arc<T>>.
/// So this structure always points to a valid sample memory, but the serdes callbacks
/// can change the value of the sample under us.
/// To be absolutely sure, I think we must put each sample into an RwLock<Arc<T>> instead of
/// an Arc<T>, I guess this is the cost we pay for zero copy.

unsafe impl<T> Send for SampleBuffer<T> {}
pub struct SampleBuffer<T> {
    /// This is !Send. This is the only way to punch through the Cyclone API as we need an array of pointers
    pub(crate) buffer: Vec<*mut Sample<T>>,
    pub(crate) sample_info: Vec<cyclonedds_sys::dds_sample_info>,
}

impl<'a, T:TopicType> SampleBuffer<T> {
    pub fn new(len: usize) -> Self {
        let mut buf = Self {
            buffer: Vec::new(),
            sample_info: vec![cyclonedds_sys::dds_sample_info::default(); len],
        };

        for _i in 0..len {
            let p = Box::into_raw(Box::default());
            buf.buffer.push(p);
        }
        buf
    }

    /// Check if sample is valid. Will panic if out of
    /// bounds.
    pub fn is_valid_sample(&self, index: usize) -> bool {
        self.sample_info[index].valid_data
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn iter(&'a self) -> impl Iterator<Item = &T> {
        let p = self.buffer.iter().filter_map(|p| {
            let sample = unsafe { &*(*p) };
            sample.try_deref()
            
        });
        p
    }

    /// Get a sample
    pub fn get(&self, index: usize) -> &Sample<T> {
        let p_sample = self.buffer[index];
        unsafe { &*p_sample }
    }

    /// return a raw pointer to the buffer and the sample info
    /// to be used in unsafe code that calls the CycloneDDS
    /// API
    pub unsafe fn as_mut_ptr(&mut self) -> (*mut *mut Sample<T>, *mut dds_sample_info) {
        (self.buffer.as_mut_ptr(), self.sample_info.as_mut_ptr())
    }
}

impl<'a, T> Drop for SampleBuffer<T> {
    fn drop(&mut self) {
        for p in &self.buffer {
            unsafe {
                let _it = Box::from_raw(*p);
            }
        }
    }
}
/*
impl <'a,T>Index<usize> for SampleBuffer<T> {
    type Output = &'a Sample<T>;
    fn index<'a>(&'a self, i: usize) -> &'a Sample<T> {
        &self.e[i]
    }
}
*/

#[allow(dead_code)]
unsafe extern "C" fn zero_samples<T>(
    _sertype: *const ddsi_sertype,
    _ptr: *mut std::ffi::c_void,
    _len: usize,
) {
} // empty implementation

#[allow(dead_code)]
extern "C" fn realloc_samples<T>(
    ptrs: *mut *mut std::ffi::c_void,
    _sertype: *const ddsi_sertype,
    old: *mut std::ffi::c_void,
    old_count: usize,
    new_count: usize,
) {
    //println!("realloc");
    let old = unsafe {
        Vec::<*mut Sample<T>>::from_raw_parts(
            old as *mut *mut Sample<T>,
            old_count as usize,
            old_count as usize,
        )
    };
    let mut new = Vec::<*mut Sample<T>>::with_capacity(new_count as usize);

    if new_count >= old_count {
        for entry in old {
            new.push(entry);
        }

        for _i in 0..(new_count - old_count) {
            new.push(Box::into_raw(Box::default()));
        }
    } else {
        for e in old.into_iter().take(new_count as usize) {
            new.push(e)
        }
    }

    let leaked = new.leak();

    let (raw, _length) = (leaked.as_ptr(), leaked.len());
    // if the length and allocated length are not equal, we messed up above.
    //assert_eq!(length, allocated_length);
    unsafe {
        *ptrs = raw as *mut std::ffi::c_void;
    }
}

#[allow(dead_code)]
extern "C" fn free_samples<T>(
    _sertype: *const ddsi_sertype,
    ptrs: *mut *mut std::ffi::c_void,
    len: usize,
    op: dds_free_op_t,
) where
    T: TopicType,
{
    let ptrs_v: *mut *mut Sample<T> = ptrs as *mut *mut Sample<T>;

    if (op & DDS_FREE_ALL_BIT) != 0 {
        let _samples =
            unsafe { Vec::<Sample<T>>::from_raw_parts(*ptrs_v, len as usize, len as usize) };
        // all samples will get freed when samples goes out of scope
    } else {
        assert_ne!(op & DDS_FREE_CONTENTS_BIT, 0);
        let mut samples =
            unsafe { Vec::<Sample<T>>::from_raw_parts(*ptrs_v, len as usize, len as usize) };
        for sample in samples.iter_mut() {
            //let _old_sample = std::mem::take(sample);
            sample.clear()
            //_old_sample goes out of scope and the content is freed. The pointer is replaced with a default constructed sample
        }
        let _intentional_leak = samples.leak();
    }
}

#[allow(dead_code)]
unsafe extern "C" fn free_sertype<T>(sertype: *mut cyclonedds_sys::ddsi_sertype) {
    ddsi_sertype_fini(sertype);

    let _sertype_ops = Box::<ddsi_sertype_ops>::from_raw((*sertype).ops as *mut ddsi_sertype_ops);
    let _serdata_ops =
        Box::<ddsi_serdata_ops>::from_raw((*sertype).serdata_ops as *mut ddsi_serdata_ops);
    // this sertype is always constructed in Rust. During destruction,
    // the Box takes over the pointer and frees it when it goes out
    // of scope.
    let sertype = sertype as *mut SerType<T>;
    let _it = Box::<SerType<T>>::from_raw(sertype);
}

// create ddsi_serdata from a fragchain
#[allow(dead_code)]
unsafe extern "C" fn serdata_from_fragchain<T>(
    sertype: *const ddsi_sertype,
    kind: u32,
    mut fragchain: *const ddsi_rdata,
    size: usize,
) -> *mut ddsi_serdata
where
    T: DeserializeOwned + TopicType,
{
    //println!("serdata_from_fragchain");
    let mut off: u32 = 0;
    let size = size as usize;
    let fragchain_ref = &*fragchain;

    let mut serdata = SerData::<T>::new(sertype, kind);

    assert_eq!(fragchain_ref.min, 0);
    assert!(fragchain_ref.maxp1 >= off);

    // The scatter gather list
    let mut sg_list = Vec::new();

    while !fragchain.is_null() {
        let fragchain_ref = &*fragchain;
        if fragchain_ref.maxp1 > off {
            let payload =
                nn_rmsg_payload_offset(fragchain_ref.rmsg, nn_rdata_payload_offset(fragchain));
            let src = payload.add((off - fragchain_ref.min) as usize);
            let n_bytes = fragchain_ref.maxp1 - off;
            sg_list.push(std::slice::from_raw_parts(src, n_bytes as usize));
            off = fragchain_ref.maxp1;
            assert!(off as usize <= size);
        }
        fragchain = fragchain_ref.nextfrag;
    }
    // make a reader out of the sg_list
    let reader = SGReader::new(&sg_list);
    if let Ok(decoded) = cdr::deserialize_from::<_, T, _>(reader, Bounded(size as u64)) {
        if T::has_key() {
            // compute the 16byte key hash
            let key_cdr = decoded.key_cdr();
            // skip the four byte header
            let key_cdr = &key_cdr[4..];
            compute_key_hash(key_cdr, &mut serdata);
        }
        serdata.serdata.hash = decoded.hash((*sertype).serdata_basehash);
        let sample = std::sync::Arc::new(decoded);
        //store the deserialized sample in the serdata. We don't need to deserialize again
        serdata.sample = SampleData::SDKData(sample);
    } else {
        println!("Deserialization error!");
        return std::ptr::null_mut();
    }

    //store the hash into the serdata

    // convert into raw pointer and forget about it (for now). Cyclone will take ownership.
    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

fn compute_key_hash<T>(key_cdr: &[u8], serdata: &mut SerData<T>)
where
    T: TopicType,
{
    let mut cdr_key = [0u8; 20];

    if T::force_md5_keyhash() || key_cdr.len() > 16 {
        let mut md5st = ddsrt_md5_state_t::default();
        let md5set = &mut md5st as *mut ddsrt_md5_state_s;
        unsafe {
            ddsrt_md5_init(md5set);
            ddsrt_md5_append(md5set, key_cdr.as_ptr(), key_cdr.len() as u32);
            ddsrt_md5_finish(md5set, cdr_key.as_mut_ptr());
        }
    } else {
        for (i, data) in key_cdr.iter().enumerate() {
            cdr_key[i] = *data;
        }
    }
    serdata.key_hash = KeyHash::CdrKey(cdr_key)
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_from_keyhash<T>(
    sertype: *const ddsi_sertype,
    keyhash: *const ddsi_keyhash,
) -> *mut ddsi_serdata
where
    T: TopicType,
{
    let keyhash = (*keyhash).value;
    //println!("serdata_from_keyhash");

    if T::force_md5_keyhash() {
        // this means keyhas fits in 16 bytes
        std::ptr::null_mut()
    } else {
        let mut serdata = SerData::<T>::new(sertype, ddsi_serdata_kind_SDK_KEY);
        serdata.sample = SampleData::SDKKey;

        let mut key_hash_buffer = [0u8; 20];
        let key_hash = &mut key_hash_buffer[4..];

        for (i, b) in keyhash.iter().enumerate() {
            key_hash[i] = *b;
        }

        serdata.key_hash = KeyHash::CdrKey(key_hash_buffer);

        let ptr = Box::into_raw(serdata);
        // only we know this ddsi_serdata is really of type SerData
        ptr as *mut ddsi_serdata
    }
}

#[allow(dead_code)]
#[allow(non_upper_case_globals)]
unsafe extern "C" fn serdata_from_sample<T>(
    sertype: *const ddsi_sertype,
    kind: u32,
    sample: *const c_void,
) -> *mut ddsi_serdata
where
    T: TopicType + 'static,
{
    //println!("Serdata from sample {:?}", sample);
    let mut serdata = SerData::<T>::new(sertype, kind);

    match kind {
        #[allow(non_upper_case_globals)]
        ddsi_serdata_kind_SDK_DATA => {
            if let Some(size) = raw_loaned_size::<T>(sample as usize) {
                // `sample` is a raw shared-memory buffer from DdsWriter::loan_of_size()/
                // loan_serialized(), already holding pre-serialized CDR bytes - not a
                // Sample<T> and not a raw *const T either. dds_write_impl calls from_sample
                // for the "normal" (non-PSMX) serdata alongside PSMX delivery even for a raw
                // loan (and this is the *only* serdata a same-process local reader ever
                // sees - PSMX/from_loaned_sample is for genuine cross-process delivery), so
                // this needs the same disambiguation from_loaned_sample's raw-loan branch
                // does, one level up: decode the bytes back into a T and store it as an
                // ordinary SDKData sample, caching the bytes we already have rather than
                // letting to_ser/get_size redundantly re-serialize them.
                let bytes = std::slice::from_raw_parts(sample as *const u8, size as usize).to_vec();
                match deserialize_type::<T>(&bytes) {
                    Ok(decoded) => {
                        serdata.serdata.hash = decoded.hash((*sertype).serdata_basehash);
                        serdata.serialized_size = Some(size);
                        serdata.cdr = Some(bytes);
                        serdata.sample = SampleData::SDKData(decoded);
                    }
                    Err(()) => {
                        println!("Deserialization error (raw loan)!");
                        return std::ptr::null_mut();
                    }
                }
            } else if is_loaned::<T>(sample as usize) {
                // `sample` is the raw *const T that DdsWriter::loan() handed to the
                // application, not a Sample<T> - see the loan_registry module comment.
                // DdsWriter::loan() only ever loans fixed-size (memcpy-safe) types, so a
                // bitwise copy out of the loan's memory is sound here.
                let owned = Arc::new(std::ptr::read(sample as *const T));
                serdata.serdata.hash = owned.hash((*sertype).serdata_basehash);
                serdata.sample = SampleData::SDKData(owned);
            } else {
                let sample = sample as *const Sample<T>;
                let sample = &*sample;
                let sample = sample.get().unwrap();
                serdata.serdata.hash = sample.hash((*sertype).serdata_basehash);
                serdata.sample = SampleData::SDKData(sample);
            }
        }
        ddsi_serdata_kind_SDK_KEY => {
            // Reached via dds_dispose/dds_writedispose/dds_unregister_instance, which pass a
            // raw *const T with (at minimum) the key fields populated - there's no safe
            // DdsWriter wrapper for these that would go through the Sample<T>/loan
            // conventions the SDK_DATA branch above has to disambiguate between, so there's
            // only one calling convention to handle here. Mirrors how from_ser/from_ser_iov/
            // from_psmx derive a key hash from a sample they already have in hand, and how
            // from_keyhash constructs an SDKKey SerData from a keyhash cyclone already
            // computed - this is the same shape, one step earlier.
            let t: &T = &*(sample as *const T);
            let key_cdr = t.key_cdr();
            // skip the four byte CDR encapsulation header, matching every other call site
            // that turns a key_cdr() into a key hash.
            compute_key_hash(&key_cdr[4..], &mut serdata);
            serdata.sample = SampleData::SDKKey;
        }
        _ => panic!("Unexpected kind"),
    }

    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_from_iov<T>(
    sertype: *const ddsi_sertype,
    kind: u32,
    niov: usize,
    iov: *const iovec,
    size: usize,
) -> *mut ddsi_serdata
where
    T: DeserializeOwned + TopicType,
{
    let size = size as usize;
    let niov = niov as usize;
    //println!("serdata_from_iov");

    let mut serdata = SerData::<T>::new(sertype, kind);

    let iovs = std::slice::from_raw_parts(iov as *const cyclonedds_sys::iovec, niov);

    let iov_slices: Vec<&[u8]> = iovs
        .iter()
        .map(|iov| {
            let iov = iov;

            std::slice::from_raw_parts(iov.iov_base as *const u8, iov.iov_len as usize)
        })
        .collect();

    // make a reader out of the sg_list
    let reader = SGReader::new(&iov_slices);

    if let Ok(decoded) = cdr::deserialize_from::<_, T, _>(reader, Bounded(size as u64)) {
        if T::has_key() {
            // compute the 16byte key hash
            let key_cdr = decoded.key_cdr();
            // skip the four byte header
            let key_cdr = &key_cdr[4..];
            compute_key_hash(key_cdr, &mut serdata);
        }
        serdata.serdata.hash = decoded.hash((*sertype).serdata_basehash);
        let sample = std::sync::Arc::new(decoded);
        //store the deserialized sample in the serdata. We don't need to deserialize again
        serdata.sample = SampleData::SDKData(sample);
    } else {
        //println!("Deserialization error!");
        return std::ptr::null_mut();
    }

    // convert into raw pointer and forget about it as ownership is passed into cyclonedds
    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

#[allow(dead_code)]
unsafe extern "C" fn free_serdata<T>(serdata: *mut ddsi_serdata) {
    //println!("free_serdata");
    // the pointer is really a *mut SerData
    let ptr = serdata as *mut SerData<T>;

    let serdata = &mut *ptr;

    if !serdata.serdata.loan.is_null() {
        // Release our reference to the loaned sample. The PSMX plugin (or the heap,
        // for a locally-loaned sample) frees it once the refcount reaches zero.
        dds_loaned_sample_unref(serdata.serdata.loan);
    }

    let _data = Box::from_raw(ptr);
    // _data goes out of scope and frees the SerData. Nothing more to do here.
}

// cdr::calc_serialized_size() computes the size before CDR's trailing padding (aligning to a
// 4-byte boundary), same as serialize_type() below has to correct for. This value becomes the
// buffer size cyclone allocates for us elsewhere (e.g. before calling serdata_to_ser, or the
// loan size DdsWriter::loan_serialized() requests), so under-reporting it means later writes
// overflow that buffer instead of merely miscounting a size hint.
pub(crate) fn padded_cdr_size<T: Serialize + ?Sized>(sample: &T) -> u32 {
    let unpadded = cdr::calc_serialized_size::<T>(sample) as u32;
    (unpadded + 3) & !3u32
}

#[allow(dead_code)]
unsafe extern "C" fn get_size<T>(serdata: *const ddsi_serdata) -> u32
where
    T: Serialize + TopicType,
{
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let size = match &serdata.sample {
        SampleData::Uninitialized => 0,
        SampleData::SDKKey => serdata.key_hash.key_length() as u32,
        // This function asks for the serialized size so we do this even for SHM Data
        SampleData::SDKData(sample) => {
            let padded = padded_cdr_size(sample.deref());
            serdata.serialized_size = Some(padded);
            padded
        }
        SampleData::SHMData(_sample) => {
            // we refuse to serialize SHM data so return 0
            0
            /*
            serdata.serialized_size = Some((cdr::calc_serialized_size::<T>(sample.as_ref())) as u32);
            *serdata.serialized_size.as_ref().unwrap()
            */
        }
    };
    size
}

#[allow(dead_code)]
unsafe extern "C" fn eqkey<T>(
    serdata_a: *const ddsi_serdata,
    serdata_b: *const ddsi_serdata,
) -> bool {
    let a = SerData::<T>::mut_ref_from_serdata(serdata_a);
    let b = SerData::<T>::mut_ref_from_serdata(serdata_b);
    a.key_hash == b.key_hash
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_ser<T>(
    serdata: *const ddsi_serdata,
    size: usize,
    offset: usize,
    buf: *mut c_void,
) where
    T: Serialize + TopicType,
{
    //println!("serdata_to_ser");
    // cyclone may call this multiple times with different (offset, size) pairs to pull
    // successive chunks out of the same serialized sample (e.g. when fragmenting a large
    // sample for the network). So the CDR encoding has to be produced once and cached, then
    // sliced - mirroring serdata_to_ser_ref below - rather than bounding the *serializer*
    // itself to `size`: the serializer has no notion of "skip the first `offset` bytes", so
    // on any call after the first it would try to fit the *whole* encoding into a buffer
    // sized for only the remaining chunk and fail (this used to panic here under load: a
    // stress test sending many samples reliably triggers the fragmented/chunked path that a
    // single small sample in earlier tests never exercised).
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let dst = buf as *mut u8;

    if size == 0 {
        return;
    }

    let copy_chunk = |src: &[u8], dst: *mut u8| {
        let start = offset.min(src.len());
        let end = (offset + size).min(src.len());
        let chunk = &src[start..end];
        std::ptr::copy_nonoverlapping(chunk.as_ptr(), dst, chunk.len());
    };

    match &serdata.sample {
        SampleData::Uninitialized => {
            panic!("Attempt to serialize uninitialized serdata")
        }
        SampleData::SDKKey => match &serdata.key_hash {
            KeyHash::None => {}
            KeyHash::CdrKey(k) => copy_chunk(k, dst),
            KeyHash::RawKey(k) => copy_chunk(k, dst),
        },
        // We may serialize both SDK data as well as SHM Data
        SampleData::SDKData(sample) => {
            if serdata.cdr.is_none() {
                serdata.cdr = serialize_type::<T>(sample, serdata.serialized_size).ok();
            }
            if let Some(cdr) = &serdata.cdr {
                copy_chunk(cdr, dst);
            } else {
                panic!("Unable to serialize type {:?}", T::typename());
            }
        }
        SampleData::SHMData(sample) => {
            if serdata.cdr.is_none() {
                serdata.cdr = serialize_type::<T>(sample.as_ref(), serdata.serialized_size).ok();
            }
            if let Some(cdr) = &serdata.cdr {
                copy_chunk(cdr, dst);
            } else {
                panic!("Unable to serialize type {:?}", T::typename());
            }
        }
    }
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_ser_ref<T>(
    serdata: *const ddsi_serdata,
    offset: usize,
    size: usize,
    iov: *mut iovec,
) -> *mut ddsi_serdata
where
    T: Serialize + TopicType,
{
    //println!("serdata_to_ser_ref");
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let iov = &mut *iov;

    match &serdata.sample {
        SampleData::Uninitialized => panic!("Attempt to serialize uninitialized Sample"),
        SampleData::SDKKey => {
            let (p, len) = match &serdata.key_hash {
                KeyHash::None => (std::ptr::null(), 0),
                KeyHash::CdrKey(k) => (k.as_ptr(), k.len()),
                KeyHash::RawKey(k) => (k.as_ptr(), k.len()),
            };

            iov.iov_base = p as *mut c_void;
            iov.iov_len = len as usize;
        }
        SampleData::SDKData(sample) => {
            if serdata.cdr.is_none() {
                serdata.cdr = serialize_type::<T>(sample, serdata.serialized_size).ok();
            }
            if let Some(cdr) = &serdata.cdr {
                let offset = offset as usize;
                let mut last = offset + size as usize;
                if last > cdr.len() - 1 {
                    last = cdr.len() - 1;
                }
                let cdr = &cdr[offset..last];
                // cdds rounds up the length into multiple of 4. We mirror that by allocating extra in the
                // ``serialize_type`` function.
                iov.iov_base = cdr.as_ptr() as *mut c_void;
                iov.iov_len = size; //cdr.len() as usize;
            } else {
                println!("Serialization error!");
                return std::ptr::null_mut();
            }
        }

        SampleData::SHMData(sample) => {
            if serdata.cdr.is_none() {
                serdata.cdr = serialize_type::<T>(sample.as_ref(), serdata.serialized_size).ok();
            }
            if let Some(cdr) = &serdata.cdr {
                let offset = offset as usize;
                let last = offset + size as usize;
                let cdr = &cdr[offset..last];
                iov.iov_base = cdr.as_ptr() as *mut c_void;
                iov.iov_len = cdr.len() as usize;
            } else {
                println!("Serialization error (SHM)!");
                return std::ptr::null_mut();
            }
        }
    }
    ddsi_serdata_addref(&serdata.serdata)
}

pub(crate) fn serialize_type<T: Serialize>(sample: &T, maybe_size: Option<u32>) -> Result<Vec<u8>, ()> {
    if let Some(size) = maybe_size {
        // Round up allocation to multiple of four
        let size = (size + 3) & !3u32;
        let mut buffer = Vec::<u8>::with_capacity(size as usize);
        if let Ok(()) = cdr::serialize_into::<_, T, _, CdrBe>(&mut buffer, sample, Infinite) {
            Ok(buffer)
        } else {
            Err(())
        }
    } else if let Ok(data) = cdr::serialize::<T, _, CdrBe>(sample, Infinite) {
        Ok(data)
    } else {
        Err(())
    }
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_ser_unref<T>(serdata: *mut ddsi_serdata, _iov: *const iovec) {
    //println!("serdata_to_ser_unref");
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    ddsi_serdata_removeref(&mut serdata.serdata)
}

fn deserialize_type<T>(data:&[u8]) -> Result<Arc<T>,()> 
    where
    T: DeserializeOwned {
        cdr::deserialize::<Box<T>>(data).map(Arc::from).map_err(|_e|())
    }

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_sample<T>(
    serdata_ptr: *const ddsi_serdata,
    sample: *mut c_void,
    _bufptr: *mut *mut c_void,
    _buflim: *mut c_void,
) -> bool
where
    T: DeserializeOwned + TopicType,
{
    //println!(
    //    "serdata to sample serdata:{:?} sample:{:?} bufptr:{:?} buflim:{:?}",
    //    serdata, sample, _bufptr, _buflim
    //);
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata_ptr);
    let mut s = Box::<Sample<T>>::from_raw(sample as *mut Sample<T>);
    assert!(!sample.is_null());

    // Every from_* constructor (from_ser, from_ser_iov, from_sample, from_loaned_sample,
    // from_psmx, ...) populates serdata.sample synchronously, so by the time we get here
    // there is nothing left to decode - just wire it up to the caller's Sample<T>.
    //
    // ddsi_serdata_to_sample_t's contract (see ddsi_serdata.h) is "return false on error" -
    // i.e. true means the sample was materialized successfully. This was inverted here
    // (false for the successful SDKData/SHMData cases), which made every single successful
    // read/take report failure back to CycloneDDS - the root cause of dds_take/dds_read
    // unconditionally returning DDS_RETCODE_ERROR regardless of topic, transport, or timing.
    let ret = match &serdata.sample {
        SampleData::Uninitialized => false,
        SampleData::SDKKey => true,
        SampleData::SDKData(_data) => {
            s.set_serdata(serdata_ptr as *mut ddsi_serdata);
            //s.set(data.clone());
            true
        }
        SampleData::SHMData(_data) => {
            s.set_serdata(serdata_ptr as *mut ddsi_serdata);
            true
        }
    };

    // leak the sample intentionally so it doesn't get deallocated here
    let _intentional_leak = Box::into_raw(s);
    ret
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_untyped<T>(serdata: *const ddsi_serdata) -> *mut ddsi_serdata {
    //println!("serdata_to_untyped {:?}", serdata);
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);

    //if let SampleData::<T>::SDKData(_d) = &serdata.sample {
    let mut untyped_serdata = SerData::<T>::new(serdata.serdata.type_, ddsi_serdata_kind_SDK_KEY);
    // untype it
    untyped_serdata.serdata.type_ = std::ptr::null_mut();
    untyped_serdata.sample = SampleData::SDKKey;

    //copy the hashes
    untyped_serdata.key_hash = serdata.key_hash.clone();
    untyped_serdata.serdata.hash = serdata.serdata.hash;

    let ptr = Box::into_raw(untyped_serdata);

    ptr as *mut ddsi_serdata
    //} else {
    //    println!("Error: Cannot convert from untyped to untyped");
    //    std::ptr::null_mut()
    //}
}

#[allow(dead_code)]
unsafe extern "C" fn untyped_to_sample<T>(
    _sertype: *const ddsi_sertype,
    _serdata: *const ddsi_serdata,
    sample: *mut c_void,
    _buf: *mut *mut c_void,
    _buflim: *mut c_void,
) -> bool
where
    T: TopicType,
{
    //println!("untyped to sample!");
    if !sample.is_null() {
        let mut sample = Box::<Sample<T>>::from_raw(sample as *mut Sample<T>);
        // hmm. We don't store serialized data in serdata. I'm not really sure how
        // to implement this. For now, invalidate the sample.
        sample.clear();
        // leak this as we don't want to deallocate it.
        let _leaked = Box::<Sample<T>>::into_raw(sample);
        true
    } else {
        false
    }
}

#[allow(dead_code)]
unsafe extern "C" fn get_keyhash<T>(
    serdata: *const ddsi_serdata,
    keyhash: *mut ddsi_keyhash,
    _force_md5: bool,
) {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let keyhash = &mut *keyhash;

    let src = match &serdata.key_hash {
        KeyHash::None => &[],
        KeyHash::CdrKey(k) => &k[4..],
        KeyHash::RawKey(k) => &k[..],
    };

    //let source_key_hash = &serdata.key_hash[4..];
    for (i, b) in src.iter().enumerate() {
        keyhash.value[i] = *b;
    }
}

#[allow(dead_code)]
unsafe extern "C" fn print<T>(
    _sertype: *const ddsi_sertype,
    _serdata: *const ddsi_serdata,
    _buf: *mut std::os::raw::c_char,
    _bufsize: usize,
) -> usize {
    0
}

fn create_sertype_ops<T>() -> Box<ddsi_sertype_ops>
where
    T: TopicType,
{
    Box::new(ddsi_sertype_ops {
        version: Some(ddsi_sertype_v0),
        arg: std::ptr::null_mut(),
        free: Some(free_sertype::<T>),
        zero_samples: Some(zero_samples::<T>),
        realloc_samples: Some(realloc_samples::<T>),
        free_samples: Some(free_samples::<T>),
        equal: Some(equal::<T>),
        hash: Some(hash::<T>),
        ..Default::default()
    })
}

// CycloneDDS 11 replaced the old iceoryx-specific get_sample_size/from_iox_buffer
// serdata_ops callbacks with a generic, PSMX-plugin-agnostic loan model. The fixed
// sample size that get_sample_size used to report is now just a plain field
// (ddsi_sertype::sizeof_type, set in SerType::new) that the middleware reads
// directly, so there's no longer a callback for it at all.
//
// from_iox_buffer's two cases (sub null: our own writer-side loan; sub non-null:
// data received from an iceoryx subscriber) are now two separate callbacks:
// from_loaned_sample (writer side) and from_psmx (reader side, any PSMX plugin).

#[cfg(feature = "shm")]
#[allow(dead_code)]
unsafe extern "C" fn from_loaned_sample<T>(
    sertype: *const ddsi_sertype,
    kind: ddsi_serdata_kind,
    _sample: *const std::os::raw::c_char,
    loaned_sample: *mut dds_loaned_sample,
    _will_require_cdr: bool,
) -> *mut ddsi_serdata
where
    T: DeserializeOwned + TopicType + 'static,
{
    if sertype.is_null() || loaned_sample.is_null() {
        return std::ptr::null_mut();
    }

    let mut d = SerData::<T>::new(sertype, kind);

    dds_loaned_sample_ref(loaned_sample);
    d.serdata.loan = loaned_sample;

    let buffer = (*loaned_sample).sample_ptr;

    if raw_loaned_size::<T>(buffer as usize).is_some() {
        // DdsWriter::loan_of_size()/loan_serialized(): the buffer already holds
        // pre-serialized CDR bytes (including the 4-byte encapsulation header), not a live
        // T. Decode it back into one - same as from_psmx's SERIALIZED_DATA case does - and
        // store it as an ordinary SampleData::SDKData: a local (same-process) reader can be
        // handed *this* serdata directly, bypassing from_psmx/PSMX entirely, and every
        // consumer (try_deref, to_sample, ...) already knows how to deal with SDKData. Cache
        // the bytes we already have in serdata.cdr/serialized_size too, so get_size/
        // serdata_to_ser* don't redundantly re-serialize what we just decoded.
        let metadata = &*(*loaned_sample).metadata;
        let size = metadata.sample_size as usize;
        let bytes = std::slice::from_raw_parts(buffer as *const u8, size).to_vec();

        match deserialize_type::<T>(&bytes) {
            Ok(decoded) => {
                if T::has_key() {
                    let key_cdr = decoded.key_cdr();
                    compute_key_hash(&key_cdr[4..], &mut d);
                }
                d.serialized_size = Some(size as u32);
                d.cdr = Some(bytes);
                d.sample = SampleData::SDKData(decoded);
            }
            Err(()) => {
                println!("Deserialization error (raw loan)!");
                return std::ptr::null_mut();
            }
        }
    } else {
        // This is our own outgoing (writer-loaned) sample: the buffer already holds a
        // plain T that the application populated directly, no (de)serialization needed.
        d.sample = SampleData::SHMData(NonNull::new_unchecked(buffer as *mut T));
    }

    let ptr = Box::into_raw(d);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

#[cfg(feature = "shm")]
#[allow(dead_code)]
unsafe extern "C" fn from_psmx<T>(
    sertype: *const ddsi_sertype,
    loaned_sample: *mut dds_loaned_sample,
) -> *mut ddsi_serdata
where
    T: DeserializeOwned + TopicType,
{
    //println!("from_psmx");

    if sertype.is_null() || loaned_sample.is_null() {
        return std::ptr::null::<ddsi_serdata>() as *mut ddsi_serdata;
    }

    let metadata = &*(*loaned_sample).metadata;
    let buffer = (*loaned_sample).sample_ptr;

    let kind = match metadata.sample_state {
        dds_loaned_sample_state_DDS_LOANED_SAMPLE_STATE_RAW_KEY
        | dds_loaned_sample_state_DDS_LOANED_SAMPLE_STATE_SERIALIZED_KEY => {
            ddsi_serdata_kind_SDK_KEY
        }
        _ => ddsi_serdata_kind_SDK_DATA,
    };

    let mut d = SerData::<T>::new(sertype, kind);

    dds_loaned_sample_ref(loaned_sample);
    d.serdata.loan = loaned_sample;

    // Unlike the old iceoryx_header, dds_psmx_metadata carries no precomputed key
    // hash, so we compute it ourselves here, same as the from_ser/from_ser_iov paths.
    //
    // metadata.sample_state alone isn't enough to tell a raw struct T from pre-serialized
    // CDR bytes: cyclone sets it to RAW_KEY/RAW_DATA for *any* outstanding writer loan once
    // dds_write() picks it up, whether it came from DdsWriter::loan() (regular, raw T) or
    // loan_of_size()/loan_serialized() (raw bytes) - and unlike from_sample/from_loaned_sample
    // (writer-side, same process), from_psmx runs in a reader process that has no access to
    // the writer's loan_registry to disambiguate the same way. T::is_fixed_size() stands in
    // for it instead, and is safe to rely on here specifically because DdsWriter::loan()
    // already refuses non-fixed-size T (see loan()), and loan_of_size() mirrors that by
    // refusing fixed-size T - so for a given T, RAW-state PSMX data can only ever have come
    // from the loan kind that type is allowed to use.
    let is_serialized = matches!(
        metadata.sample_state,
        dds_loaned_sample_state_DDS_LOANED_SAMPLE_STATE_SERIALIZED_KEY
            | dds_loaned_sample_state_DDS_LOANED_SAMPLE_STATE_SERIALIZED_DATA
    ) || !T::is_fixed_size();

    if is_serialized {
        let reader =
            std::slice::from_raw_parts(buffer as *const u8, metadata.sample_size as usize);
        if kind == ddsi_serdata_kind_SDK_KEY {
            compute_key_hash(reader, &mut d);
            d.sample = SampleData::SDKKey;
        } else if let Ok(decoded) = deserialize_type::<T>(reader) {
            if T::has_key() {
                // compute the 16byte key hash, skipping the four byte CDR header
                let key_cdr = decoded.key_cdr();
                let key_cdr = &key_cdr[4..];
                compute_key_hash(key_cdr, &mut d);
            }
            d.sample = SampleData::SDKData(decoded);
        } else {
            println!("Deserialization error!");
            return std::ptr::null_mut();
        }
    } else {
        // Raw (unserialized) data: the loan's memory already holds a plain T. Only
        // reachable when T::is_fixed_size() - see is_serialized above.
        if T::has_key() {
            let key_cdr = (&*(buffer as *const T)).key_cdr();
            let key_cdr = &key_cdr[4..];
            compute_key_hash(key_cdr, &mut d);
        }
        d.sample = SampleData::SHMData(NonNull::new_unchecked(buffer as *mut T));
    }

    let ptr = Box::into_raw(d);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

fn create_serdata_ops<T>() -> Box<ddsi_serdata_ops>
where
    T: DeserializeOwned + TopicType + Serialize + 'static,
{
    Box::new(ddsi_serdata_ops {
        eqkey: Some(eqkey::<T>),
        get_size: Some(get_size::<T>),
        from_ser: Some(serdata_from_fragchain::<T>),
        from_ser_iov: Some(serdata_from_iov::<T>),
        from_keyhash: Some(serdata_from_keyhash::<T>),
        from_sample: Some(serdata_from_sample::<T>),
        to_ser: Some(serdata_to_ser::<T>),
        to_ser_ref: Some(serdata_to_ser_ref::<T>),
        to_ser_unref: Some(serdata_to_ser_unref::<T>),
        to_sample: Some(serdata_to_sample::<T>),
        to_untyped: Some(serdata_to_untyped::<T>),
        untyped_to_sample: Some(untyped_to_sample::<T>),
        free: Some(free_serdata::<T>),
        print: Some(print::<T>),
        get_keyhash: Some(get_keyhash::<T>),
        #[cfg(feature = "shm")]
        from_loaned_sample: Some(from_loaned_sample::<T>),
        #[cfg(feature = "shm")]
        from_psmx: Some(from_psmx::<T>),
        ..Default::default()
    })
}

// not sure what this needs to do. The C++ implementation at
// https://github.com/eclipse-cyclonedds/cyclonedds-cxx/blob/templated-streaming/src/ddscxx/include/org/eclipse/cyclonedds/topic/datatopic.hpp
// just returns 0
// Update! : Now I understand this after debugging crashes when stress testing
// with a large number of types being published. This hash is used as the hash
// lookup in hopscotch.c. 
// /*
//  * The hopscotch hash table is dependent on a proper functioning hash.
//  * If the hash function generates a lot of hash collisions, then it will
//  * not be able to handle that by design.
//  * It is capable of handling some collisions, but not more than 32 per
//  * bucket (less, when other hash values are clustered around the
//  * collision value).
//  * When proper distributed hash values are generated, then hopscotch
//  * works nice and quickly.
//  */

unsafe extern "C" fn hash<T: TopicType>(tp: *const ddsi_sertype) -> u32  
{
    if let Some(ser_type) = SerType::<T>::try_from_sertype(tp) {
        let type_name =  CStr::from_ptr(ser_type.sertype.type_name);
        let type_name_bytes = type_name.to_bytes();
        let type_size = core::mem::size_of::<T>().to_ne_bytes();
        let sg_list = [type_name_bytes,&type_size];
        let mut sg_buffer = SGReader::new(&sg_list);

        let hash = murmur3_32(&mut sg_buffer, 0);
        
        let _intentional_leak = SerType::<T>::into_sertype(ser_type);
        hash.unwrap_or(0)

    } else {
        0
    }
}

unsafe extern "C" fn equal<T>(acmn: *const ddsi_sertype, bcmn: *const ddsi_sertype) -> bool {
    let acmn = CStr::from_ptr((*acmn).type_name as *mut std::os::raw::c_char);
    let bcmn = CStr::from_ptr((*bcmn).type_name as *mut std::os::raw::c_char);
    acmn == bcmn
}

#[derive(Clone)]
enum SampleData<T> {
    Uninitialized,
    SDKKey,
    SDKData(std::sync::Arc<T>),
    SHMData(NonNull<T>),
}

impl<T> Default for SampleData<T> {
    fn default() -> Self {
        Self::Uninitialized
    }
}


#[derive(PartialEq, Clone)]
enum KeyHash {
    None,
    CdrKey([u8; 20]),
    RawKey([u8; 16]),
}

impl Default for KeyHash {
    fn default() -> Self {
        Self::None
    }
}

impl KeyHash {
    fn get_key_hash(&self) -> &[u8] {
        match self {
            KeyHash::None => &[],
            KeyHash::CdrKey(cdr_key_hash) => cdr_key_hash,
            KeyHash::RawKey(raw_key_hash) => raw_key_hash,
        }
    }
    fn key_length(&self) -> usize {
        match self {
            KeyHash::CdrKey(k) => k.len(),
            KeyHash::RawKey(k) => k.len(),
            _ => 0,
        }
    }
}

/// A representation for the serialized data.
#[repr(C)]
pub (crate)struct SerData<T> {
    serdata: ddsi_serdata,
    sample: SampleData<T>,
    //data in CDR format. This is put into an option as we only create
    //the serialized version when we need it
    cdr: Option<Vec<u8>>,
    //key_hash: ddsi_keyhash,
    // include 4 bytes of CDR encapsulation header
    //key_hash: [u8; 20],
    key_hash: KeyHash,
    // We store the serialized size here if available
    serialized_size: Option<u32>,
}

impl<'a, T> SerData<T> {
    fn new(sertype: *const ddsi_sertype, kind: u32) -> Box<SerData<T>> {
        Box::<SerData<T>>::new(SerData {
            serdata: {
                let mut data = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_serdata_init(data.as_mut_ptr(), sertype, kind);
                    data.assume_init()
                }
            },
            sample: SampleData::default(),
            cdr: None,
            key_hash: KeyHash::default(),
            serialized_size: None,
        })
    }

    fn const_ref_from_serdata(serdata: *const ddsi_serdata) -> &'a Self {
        let ptr = serdata as *const SerData<T>;
        unsafe { &*ptr }
    }

    fn mut_ref_from_serdata(serdata: *const ddsi_serdata) -> &'a mut Self {
        let ptr = serdata as *mut SerData<T>;
        unsafe { &mut *ptr }
    }
}

impl <T>Clone for SerData<T> {
    fn clone(&self) -> Self {
        Self { 
                serdata: {
                    let mut newdata = self.serdata;
                    unsafe {ddsi_serdata_addref(&mut newdata)};
                    newdata
                }, sample:  match &self.sample {
                        SampleData::Uninitialized => SampleData::Uninitialized,
                        SampleData::SDKKey => SampleData::SDKKey,
                        SampleData::SDKData(d) => SampleData::SDKData(d.clone()),
                        SampleData::SHMData(d) => SampleData::SHMData(*d),
                    }, cdr: self.cdr.clone(), key_hash: self.key_hash.clone(), serialized_size: self.serialized_size }
    }
} 



/*  These functions are created from the macros in
    https://github.com/eclipse-cyclonedds/cyclonedds/blob/f879dc0ef56eb00857c0cbb66ee87c577ff527e8/src/core/ddsi/include/dds/ddsi/q_radmin.h#L108
    Bad things will happen if these macros change.
    Some discussions here: https://github.com/eclipse-cyclonedds/cyclonedds/issues/830
*/
fn nn_rdata_payload_offset(rdata: *const ddsi_rdata) -> usize {
    unsafe { (*rdata).payload_zoff as usize }
}

fn nn_rmsg_payload(rmsg: *const ddsi_rmsg) -> *const u8 {
    unsafe { rmsg.add(1) as *const u8 }
}

fn nn_rmsg_payload_offset(rmsg: *const ddsi_rmsg, offset: usize) -> *const u8 {
    unsafe { nn_rmsg_payload(rmsg).add(offset) }
}

/// A reader for a list of scatter gather buffers
struct SGReader<'a> {
    sc_list: Option<  &'a[&'a [u8]]>,
    //the current slice that is used
    slice_cursor: usize,
    //the current offset within the slice
    slice_offset: usize,
}

impl<'a> SGReader<'a> {
    pub fn new(sc_list: &'a[&'a [u8]]) -> Self {
        SGReader {
            sc_list: Some(sc_list),
            slice_cursor: 0,
            slice_offset: 0,
        }
    }
}

impl<'a> Read for SGReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read_buf_len = buf.len();
        if self.sc_list.is_some() {
            let source_slice = self.sc_list.as_ref().unwrap()[self.slice_cursor];
            let num_slices = self.sc_list.as_ref().unwrap().len();
            let source_slice_rem = source_slice.len() - self.slice_offset;
            let source_slice = &source_slice[self.slice_offset..];

            let copy_length = std::cmp::min(source_slice_rem, read_buf_len);

            //copy the bytes, lengths have to be the same
            buf[..copy_length].copy_from_slice(&source_slice[..copy_length]);

            if copy_length == source_slice_rem {
                // we have completed this slice. move to the next
                self.slice_cursor += 1;
                self.slice_offset = 0;

                if self.slice_cursor >= num_slices {
                    //no more slices, invalidate the sc_list
                    let _ = self.sc_list.take();
                }
            } else {
                // we have not completed the current slice, just bump up the slice offset
                self.slice_offset += copy_length;
            }

            Ok(copy_length)
        } else {
            // No more data
            Ok(0)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{DdsListener, DdsParticipant, DdsQos, DdsTopic};
    use cdds_derive::Topic;
    use serde_derive::{Deserialize, Serialize};
    use std::ffi::CString;

    #[test]
    fn loan_registry_tracks_mark_and_unmark() {
        struct MarkerA;

        let addr = 0x1000usize;
        assert!(!is_loaned::<MarkerA>(addr));

        mark_loaned::<MarkerA>(addr);
        assert!(is_loaned::<MarkerA>(addr));

        unmark_loaned::<MarkerA>(addr);
        assert!(!is_loaned::<MarkerA>(addr));
    }

    #[test]
    fn loan_registry_is_isolated_per_type() {
        struct MarkerB;
        struct MarkerC;

        // The same numeric address means nothing on its own - the registry is keyed by
        // TypeId, matching how it's actually used (from_sample only knows T, not which
        // writer/loan pool an address came from).
        let addr = 0x2000usize;

        mark_loaned::<MarkerB>(addr);
        assert!(is_loaned::<MarkerB>(addr));
        assert!(!is_loaned::<MarkerC>(addr));

        unmark_loaned::<MarkerB>(addr);
        assert!(!is_loaned::<MarkerB>(addr));
    }

    #[test]
    fn loan_registry_unmark_of_unmarked_address_is_a_no_op() {
        struct MarkerD;

        // Must not panic - Loaned<T>::drop() always calls unmark_loaned, including for
        // loans that were returned uninitialized (never handed to dds_write) or for a
        // type that never had anything marked at all.
        unmark_loaned::<MarkerD>(0x3000usize);
        assert!(!is_loaned::<MarkerD>(0x3000usize));
    }

    #[test]
    fn scatter_gather() {
        let a = vec![1, 2, 3, 4, 5, 6];
        let b = vec![7, 8, 9, 10, 11];
        let c = vec![12, 13, 14, 15];
        let d = vec![16, 17, 18, 19, 20, 21];

        let sla = unsafe { std::slice::from_raw_parts(a.as_ptr(), a.len()) };
        let slb = unsafe { std::slice::from_raw_parts(b.as_ptr(), b.len()) };
        let slc = unsafe { std::slice::from_raw_parts(c.as_ptr(), c.len()) };
        let sld = unsafe { std::slice::from_raw_parts(d.as_ptr(), d.len()) };

        let sc_list = vec![sla, slb, slc, sld];

        let mut reader = SGReader::new(&sc_list);

        let mut buf = vec![0, 0, 0, 0, 0];
        if let Ok(n) = reader.read(&mut buf) {
            assert_eq!(&buf[..n], vec![1, 2, 3, 4, 5]);
        } else {
            panic!("should not panic");
        }
        if let Ok(n) = reader.read(&mut buf) {
            assert_eq!(&buf[..n], vec![6]);
        } else {
            panic!("should not panic");
        }
    }

    #[test]
    fn keyhash_basic() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            y: u32,
        }
        let foo = Foo {
            id: 0x12345678,
            x: 10,
            y: 20,
        };
        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr, vec![0, 0, 0, 0, 0x12u8, 0x34u8, 0x56u8, 0x78u8]);
    }
    #[test]
    fn keyhash_simple() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            #[topic_key]
            s: String,
            y: u32,
        }
        let foo = Foo {
            id: 0x12345678,
            x: 10,
            s: String::from("boo"),
            y: 20,
        };
        let key_cdr = foo.key_cdr();
        assert_eq!(
            key_cdr,
            vec![0, 0, 0, 0, 18, 52, 86, 120, 0, 0, 0, 4, 98, 111, 111, 0]
        );
    }

    #[test]
    fn keyhash_nested() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct NestedFoo {
            name: String,
            val: u64,
            #[topic_key]
            instance: u32,
        }

        assert_eq!(
            NestedFoo::typename(),
            CString::new("serdes::test::keyhash_nested::NestedFoo").unwrap()
        );

        impl NestedFoo {
            fn new() -> Self {
                Self {
                    name: "my name".to_owned(),
                    val: 42,
                    instance: 25,
                }
            }
        }

        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            #[topic_key]
            s: String,
            y: u32,
            #[topic_key]
            inner: NestedFoo,
        }
        let foo = Foo {
            id: 0x12345678,
            x: 10,
            s: String::from("boo"),
            y: 20,
            inner: NestedFoo::new(),
        };
        let key_cdr = foo.key_cdr();
        assert_eq!(
            key_cdr,
            vec![0, 0, 0, 0, 18, 52, 86, 120, 0, 0, 0, 4, 98, 111, 111, 0, 0, 0, 0, 25]
        );
    }

    #[test]
    fn primitive_array_as_key() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            a: [u8; 8],
            b: u32,
            c: String,
        }

        let foo = Foo {
            a: [0, 0, 0, 0, 0, 0, 0, 0],
            b: 42,
            c: "foo".to_owned(),
        };

        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr, vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(false, Foo::force_md5_keyhash());
    }

    #[test]
    fn primitive_array_and_string_as_key() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            a: [u8; 8],
            b: u32,
            #[topic_key]
            c: String,
        }

        let foo = Foo {
            a: [0, 0, 0, 0, 0, 0, 0, 0],
            b: 42,
            c: "foo".to_owned(),
        };

        let key_cdr = foo.key_cdr();
        assert_eq!(
            key_cdr,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 102, 111, 111, 0]
        );
        assert_eq!(true, Foo::force_md5_keyhash());
    }

    #[test]
    fn basic() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct NestedFoo {
            name: String,
            val: u64,
            #[topic_key]
            instance: u32,
        }

        impl NestedFoo {
            fn new() -> Self {
                Self {
                    name: "my name".to_owned(),
                    val: 42,
                    instance: 25,
                }
            }
        }

        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            #[topic_key]
            s: String,
            y: u32,
            #[topic_key]
            inner: NestedFoo,
        }
        let _foo = Foo {
            id: 0x12345678,
            x: 10,
            s: String::from("boo"),
            y: 20,
            inner: NestedFoo::new(),
        };
        let t = SerType::<Foo>::new();
        let mut t = SerType::into_sertype(t);
        let tt = &mut t as *mut *mut ddsi_sertype;
        unsafe {
            let p = dds_create_participant(0, std::ptr::null_mut(), std::ptr::null_mut());
            let topic_name = CString::new("topic_name").unwrap();
            let topic = dds_create_topic_sertype(
                p,
                topic_name.as_ptr(),
                tt,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );

            dds_delete(topic);
        }
    }
}
