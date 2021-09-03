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
use serde::{Serialize, de::DeserializeOwned};
use std::io::prelude::*;
use std::sync::RwLock;
use std::{
    ffi::{c_void, CStr},
    marker::PhantomData,
    ops::{Deref},
    sync::Arc,
};

use std::hash::Hasher;


use cyclonedds_sys::*;
//use fasthash::{murmur3::Hasher32, FastHasher};
use murmur3::murmur3_32;
use std::io::Cursor;


#[repr(C)]
pub struct SerType<T> {
    sertype: ddsi_sertype,
    _phantom: PhantomData<T>,
}

pub trait TopicType : Default + Serialize + DeserializeOwned {
    // generate a non-cryptographic hash of the key values to be used internally
    // in cyclonedds
    fn hash(&self) -> u32 {
        let cdr =  self.key_cdr();
        let mut cursor = Cursor::new(cdr.as_slice());
        murmur3_32(&mut cursor,0).unwrap()
    }

    /// The type name for this topic
    fn typename() -> std::ffi::CString {
        let ty_name_parts : String = std::any::type_name::<Self>().to_string().split("::").skip(1).collect::<Vec<_>>().join("::");

        let typename = std::ffi::CString::new(ty_name_parts)
            .expect("Unable to create CString for type name");
        println!("Typename:{:?}",&typename);
        typename
    }

    /// The default topic_name to use when creating a topic of this type. The default
    /// implementation uses '/' instead of '::' to form a unix like path.
    /// A prefix can optionally be added
    fn topic_name(maybe_prefix : Option<&str>) -> String {
        let topic_name_parts : String = format!("/{}",std::any::type_name::<Self>().to_string().split("::").skip(1).collect::<Vec<_>>().join("/"));
        

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
        T: Default + DeserializeOwned + Serialize + TopicType,
    {
        Box::<SerType<T>>::new(SerType {
            sertype: {
                let mut sertype = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_sertype_init(
                        sertype.as_mut_ptr(),
                        T::typename().as_ptr(),
                        Box::into_raw(create_sertype_ops::<T>()),
                        Box::into_raw(create_serdata_ops::<T>()),
                        !T::has_key(),
                    );
                    sertype.assume_init()
                }
            },
            _phantom: PhantomData,
        })
    }

    // cast into cyclone dds sertype.  Rust relinquishes ownership here. 
    // Cyclone DDS will free this. But if you need to free this pointer
    // before handing it over to cyclone, make sure you explicitly free it
    pub fn into_sertype(sertype: Box::<SerType<T>>) -> *mut ddsi_sertype {
        Box::<SerType<T>>::into_raw(sertype) as *mut ddsi_sertype
    }
}

/// A sample is simply a smart pointer. The storage for the sample
/// is in the serdata structure.

pub struct Sample<T> {
    sample : RwLock<Option<Arc<T>>>,
}

impl <T> Sample <T> {
    pub fn get(&self) -> Option<Arc<T>> {
        let t = self.sample.read().unwrap();
        match &*t {
            None => None,
            Some(t) => {
                Some(t.clone())},
        }
    }

    pub fn set(&mut self, t: Arc<T>) {
        let mut sample = self.sample.write().unwrap();
        sample.replace(t);
    }

    pub fn clear(&mut self) {
        let mut sample = self.sample.write().unwrap();
        let _t = sample.take();
    }

 
    pub fn from(it: Arc<T>) -> Self {
        //Self::Value(it)
        Self {
            sample : RwLock::new(Some(it)),
        }
    }
    
}

impl <T> Default for Sample<T> {
    fn default() -> Self {
        Self {
            sample : RwLock::new(None),
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

unsafe impl <T> Send for SampleBuffer<T> {}
pub struct SampleBuffer<T> {
    /// This is !Send. This is the only way to punch through the Cyclone API as we need an array of pointers
    buffer : Vec<*mut Sample<T>>,
    sample_info : Vec<cyclonedds_sys::dds_sample_info>,
}

impl <'a,T>SampleBuffer<T> {
    pub fn new(len: usize) -> Self {
        let mut buf = Self {
            buffer: Vec::new(),
            sample_info : vec![cyclonedds_sys::dds_sample_info::default();len],
        };

        for _i in 0..len {
            let p = Box::into_raw(Box::new(Sample::<T>::default()));
            buf.buffer.push(p);
        }
        buf
    }

    /// Check if sample is valid. Will panic if out of
    /// bounds.
    pub fn is_valid_sample(&self,index:usize) -> bool {
        self.sample_info[index].valid_data
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn iter(&'a self) -> impl Iterator<Item = &Sample<T>>  {
        let p = self.buffer.iter().filter_map(|p| {
            let sample =  unsafe{&*(*p)};
            let s = sample.sample.read().unwrap();
            match *s {
                Some(_) => Some(sample),
                None => None,
            }
    });
        p
    }

    /// Get a sample
    pub fn get(&self, index : usize) -> &Sample<T> {
        let p_sample = self.buffer[index];
        unsafe {&*p_sample}
    }

    /// return a raw pointer to the buffer and the sample info
    /// to be used in unsafe code that calls the CycloneDDS
    /// API 
    pub unsafe fn as_mut_ptr(&mut self) -> (*mut *mut Sample<T>,*mut dds_sample_info){
        (self.buffer.as_mut_ptr(),self.sample_info.as_mut_ptr())
    }
}

impl <'a,T> Drop for SampleBuffer<T> {
    fn drop(&mut self) {
       for p in &self.buffer {
           unsafe {Box::from_raw(*p);}
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
    _len: u64,
) {
} // empty implementation

#[allow(dead_code)]
extern "C" fn realloc_samples<T>(
    ptrs: *mut *mut std::ffi::c_void,
    _sertype: *const ddsi_sertype,
    old: *mut std::ffi::c_void,
    old_count: u64,
    new_count: u64,
) {
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
            new.push(Box::into_raw(Box::new(Sample::<T>::default())));
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
    len: u64,
    op: dds_free_op_t,
) where
    T: Default,
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
            let _old_sample = std::mem::take(sample);
            //_old_sample goes out of scope and the content is freed. The pointer is replaced with a default constructed sample
        }
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
    Box::<SerType<T>>::from_raw(sertype);
}

// create ddsi_serdata from a fragchain
#[allow(dead_code)]
unsafe extern "C" fn serdata_from_fragchain<T>(
    sertype: *const ddsi_sertype,
    kind: u32,
    mut fragchain: *const nn_rdata,
    size: u64,
) -> *mut ddsi_serdata
where
    T: DeserializeOwned + TopicType,
{
    let mut off: u32 = 0;
    let size = size as usize;
    let fragchain_ref = &*fragchain;

    let mut serdata = SerData::<T>::new(sertype, kind);

    assert_eq!(fragchain_ref.min, 0);
    assert!(fragchain_ref.maxp1 >= off as u32);

    // The scatter gather list
    let mut sg_list = Vec::new();

    while !fragchain.is_null() {
        let fragchain_ref = &*fragchain;
        if fragchain_ref.maxp1 > off as u32 {
            let payload =
                nn_rmsg_payload_offset(fragchain_ref.rmsg, nn_rdata_payload_offset(fragchain));
            let src = payload.add((off - fragchain_ref.min) as usize);
            let n_bytes = fragchain_ref.maxp1 - off as u32;
            sg_list.push(std::slice::from_raw_parts(src, n_bytes as usize));
            off = fragchain_ref.maxp1;
            assert!(off as usize <= size);
        }
        fragchain = fragchain_ref.nextfrag;
    }
    //let len : usize = sg_list.iter().fold(0usize, |s,e| s + e.len() );
    //println!("Fragchain: elements:{} {} bytes",sg_list.len(),len );
    // make a reader out of the sg_list
    let reader = SGReader::new(sg_list);
    if let Ok(decoded) = cdr::deserialize_from::<_, T, _>(reader, Bounded(size as u64)) {
        if T::has_key() {
            serdata.serdata.hash = decoded.hash();
            // compute the 16byte key hash
            let key_cdr = decoded.key_cdr();
            // skip the four byte header
            let key_cdr = &key_cdr[4..];
            compute_key_hash(key_cdr, &mut serdata);
        }
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

fn compute_key_hash <T>(key_cdr: &[u8], serdata: &mut Box<SerData<T>>) where T: TopicType {
    if T::force_md5_keyhash() || key_cdr.len() > 16 {
        let mut md5st = ddsrt_md5_state_t::default(); 
        let md5set = &mut md5st as *mut ddsrt_md5_state_s;
        unsafe {
            ddsrt_md5_init(md5set);
            ddsrt_md5_append(md5set, key_cdr.as_ptr(), key_cdr.len() as u32);
            ddsrt_md5_finish(md5set, serdata.key_hash.as_mut_ptr());
        }
    } else {
        serdata.key_hash.fill(0);
        for (i,data) in key_cdr.iter().enumerate() {
            serdata.key_hash[i] = *data;
        }
    }
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_from_keyhash<T>(
    sertype: *const ddsi_sertype,
    keyhash: *const ddsi_keyhash,
) -> *mut ddsi_serdata where T: TopicType {
 
    let keyhash = (*keyhash).value;
    
    if T::force_md5_keyhash() {
        // this means keyhas fits in 16 bytes
        std::ptr::null_mut()
    } else {
        let mut serdata = SerData::<T>::new(sertype, ddsi_serdata_kind_SDK_KEY);
        serdata.sample = SampleData::SDKKey;
        let key_hash = &mut serdata.key_hash[4..];

        for (i,b) in keyhash.iter().enumerate() {
            key_hash[i] = *b;
        }

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
) -> *mut ddsi_serdata {
    
    let mut serdata = SerData::<T>::new(sertype, kind);
    let sample = sample as *const Sample<T>;
    let sample = &*sample;

    match kind {
        #[allow(non_upper_case_globals)]
        ddsi_serdata_kind_SDK_DATA => {
            serdata.sample = SampleData::SDKData(sample.get().unwrap());
        }

        ddsi_serdata_kind_SDK_KEY => {
            panic!("Don't know how to create serdata from sample for SDK_KEY");
        }

        _ => panic!("Unexpected kind")
    }

    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_from_iov<T>(
    sertype: *const ddsi_sertype,
    kind: u32,
    niov: u64,
    iov: *const iovec,
    size: u64,
) -> *mut ddsi_serdata
where
    T: DeserializeOwned + TopicType,
{
    let size = size as usize;
    let niov = niov as usize;

    let mut serdata = SerData::<T>::new(sertype, kind);

    let iovs = std::slice::from_raw_parts(iov as *mut *const cyclonedds_sys::iovec, niov as usize);

    let iov_slices: Vec<&[u8]> = iovs
        .iter()
        .map(|iov| {
            let iov = &**iov;
            
            std::slice::from_raw_parts(iov.iov_base as *const u8, iov.iov_len as usize)
        })
        .collect();

    // make a reader out of the sg_list
    let mut reader = SGReader::new(iov_slices);
    // skip the cdr_encoding options that cyclone inserts.
    let mut cdr_encoding_options = [0u8; 4];
    let _ = reader.read_exact(&mut cdr_encoding_options);

    if let Ok(decoded) = cdr::deserialize_from::<_, T, _>(reader, Bounded(size as u64)) {
        if T::has_key() {
            serdata.serdata.hash = decoded.hash();
            // compute the 16byte key hash
            let key_cdr = decoded.key_cdr();
            // skip the four byte header
            let key_cdr = &key_cdr[4..];
            compute_key_hash(key_cdr, &mut serdata);
        }
        let sample = std::sync::Arc::new(decoded);
        //store the deserialized sample in the serdata. We don't need to deserialize again
        serdata.sample = SampleData::SDKData(sample);
    } else {
        println!("Deserialization error!");
        return std::ptr::null_mut();
    }

    // convert into raw pointer and forget about it as ownership is passed into cyclonedds
    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

#[allow(dead_code)]
unsafe extern "C" fn free_serdata<T>(serdata: *mut ddsi_serdata) {
    // the pointer is really a *mut SerData
    let ptr = serdata as *mut SerData<T>;
    let _data = Box::from_raw(ptr);
    // _data goes out of scope and frees the SerData. Nothing more to do here.
}

#[allow(dead_code)]
unsafe extern "C" fn get_size<T>(serdata: *const ddsi_serdata) -> u32
where
    T: Serialize,
{
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    match &serdata.sample {
        SampleData::Uninitialized =>panic!("Uninitialized SerData. no size possible"),
        SampleData::SDKKey => serdata.key_hash.len() as u32,
        SampleData::SDKData(serdata) => (cdr::calc_serialized_size(&serdata.deref())) as u32,
    }
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
    size: u64,
    offset: u64,
    buf: *mut c_void,
) where
    T: Serialize + TopicType,
{
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    let buf = buf as *mut u8;
    let buf = buf.add(offset as usize);

    match &serdata.sample {
        SampleData::Uninitialized => { panic!("Attempt to serialize uninitialized serdata")},
        SampleData::SDKKey => {
            // just copy the key hash that is already serialized. This includes the four byte CDR encapsulation header
            std::ptr::copy_nonoverlapping(serdata.key_hash.as_ptr(), buf, size as usize);
        },
        SampleData::SDKData(serdata) => {
            
            let buf_slice = std::slice::from_raw_parts_mut(buf, size as usize);
            if let Err(e) =
                cdr::serialize_into::<_, T, _, CdrBe>(buf_slice, serdata.deref(), Bounded(size as u64))
            {
                panic!("Unable to serialize type {:?} due to {}", T::typename(), e);
            }
        },
    }
    
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_ser_ref<T>(
    serdata: *const ddsi_serdata,
    offset: u64,
    size: u64,
    iov: *mut iovec,
) -> *mut ddsi_serdata
where
    T: Serialize + TopicType,
{
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let iov = &mut *iov;

    match &serdata.sample {
        SampleData::Uninitialized => panic!("Attempt to serialize uninitialized Sample"),
        SampleData::SDKKey => {
            iov.iov_base = serdata.key_hash.as_ptr() as *mut c_void;
            iov.iov_len = serdata.key_hash.len() as u64;
        }
        SampleData::SDKData(sample) => {
            if serdata.cdr.is_none() {
                if let Ok(data) = cdr::serialize::<T, _, CdrBe>(sample.as_ref(), Infinite) {
                    serdata.cdr = Some(data);
                } else {
                    panic!("Unable to serialize type {:?} due to", T::typename());
                }
            }
            if let Some(cdr) = &serdata.cdr {
                let offset = offset as usize;
                let last = offset + size as usize;
                let cdr = &cdr[offset..last];
                iov.iov_base = cdr.as_ptr() as *mut c_void;
                iov.iov_len = cdr.len() as u64;
            } else {
                panic!("Unexpected");
            }
        },
    }
    ddsi_serdata_addref(&serdata.serdata)
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_ser_unref<T>(serdata: *mut ddsi_serdata, _iov: *const iovec) {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    ddsi_serdata_removeref(&mut serdata.serdata)
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_sample<T>(
    serdata: *const ddsi_serdata,
    sample: *mut c_void,
    _bufptr: *mut *mut c_void,
    _buflim: *mut c_void,
) -> bool {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    //let sample = &mut *(sample as *mut Sample<T>);
    let mut s = Box::<Sample<T>>::from_raw(sample as *mut Sample<T>);
    let ret = if let SampleData::SDKData(data) = &serdata.sample {
        s.set(data.clone()) ;
        false
    } else {
        true
    };

    let _intentional_leak = Box::into_raw(s);
    ret

    
}

#[allow(dead_code)]
unsafe extern "C" fn serdata_to_untyped<T>(serdata: *const ddsi_serdata) -> *mut ddsi_serdata {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    
    if let SampleData::<T>::SDKData(_d) = &serdata.sample {
        let mut untyped_serdata = SerData::<T>::new(serdata.serdata.type_, ddsi_serdata_kind_SDK_KEY);
        // untype it
        untyped_serdata.serdata.type_ = std::ptr::null_mut();
        untyped_serdata.sample = SampleData::SDKKey;

        //copy the hashes
        untyped_serdata.key_hash = serdata.key_hash;
        untyped_serdata.serdata.hash = serdata.serdata.hash;

        let ptr = Box::into_raw(untyped_serdata);
        ptr as *mut ddsi_serdata

    } else {
        println!("Error: Cannot convert from untyped to untyped");
        std::ptr::null_mut()
    }
}

#[allow(dead_code)]
unsafe extern "C" fn untyped_to_sample<T>(_sertype: *const ddsi_sertype, 
    _serdata: *const ddsi_serdata, 
    sample : *mut c_void, 
    _buf: *mut *mut c_void, 
    _buflim: *mut c_void) -> bool {

    if !sample.is_null() {
        let mut sample = Box::<Sample<T>>::from_raw(sample as *mut Sample<T>);
        // hmm. We don't store serialized data in serdata. I'm not really sure how
        // to implement this. For now, invalidate the sample.
        sample.clear();
        true

    } else {
        false
    }
}

#[allow(dead_code)]
unsafe extern "C" fn get_keyhash<T>(serdata: *const ddsi_serdata, keyhash: *mut ddsi_keyhash, _force_md5: bool) {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let keyhash = &mut *keyhash;
    let source_key_hash = &serdata.key_hash[4..];
    for (i,b) in source_key_hash.iter().enumerate() {
        keyhash.value[i] = *b;
    }
    
}

#[allow(dead_code)]
unsafe extern "C" fn print<T>(_sertype: *const ddsi_sertype, _serdata:  *const ddsi_serdata, _buf:  *mut std::os::raw::c_char, _bufsize: u64) -> u64 {
    0
}


fn create_sertype_ops<T>() -> Box<ddsi_sertype_ops>
where
    T: Default,
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
        typeid_hash: None,
        serialized_size: None,
        serialize: None,
        deserialize: None,
        assignable_from: None,
    })
}

fn create_serdata_ops<T>() -> Box<ddsi_serdata_ops>
where
    T: DeserializeOwned + TopicType + Serialize,
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
    })
}

// not sure what this needs to do. The C++ implementation at
// https://github.com/eclipse-cyclonedds/cyclonedds-cxx/blob/templated-streaming/src/ddscxx/include/org/eclipse/cyclonedds/topic/datatopic.hpp
// just returns 0
unsafe extern "C" fn hash<T>(_acmn: *const ddsi_sertype) -> u32 {
    0
}

unsafe extern "C" fn equal<T>(acmn: *const ddsi_sertype, bcmn: *const ddsi_sertype) -> bool {
    let acmn = CStr::from_ptr((*acmn).type_name as *mut std::os::raw::c_char);
    let bcmn = CStr::from_ptr((*bcmn).type_name as *mut std::os::raw::c_char);
    acmn == bcmn
}

enum SampleData<T> {
    Uninitialized,
    SDKKey,
    SDKData(std::sync::Arc<T>),
}

impl<T> Default for SampleData<T> {
    fn default() -> Self {
        Self::Uninitialized
    }
}

/// A representation for the serialized data.
#[repr(C)]
struct SerData<T> {
    serdata: ddsi_serdata,
    sample: SampleData<T>,
    //data in CDR format. This is put into an option as we only create
    //the serialized version when we need it
    cdr: Option<Vec<u8>>,
    //key_hash: ddsi_keyhash,
    // include 4 bytes of CDR encapsulation header
    key_hash : [u8;20],
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
            key_hash: [0;20],
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

/*  These functions are created from the macros in
    https://github.com/eclipse-cyclonedds/cyclonedds/blob/f879dc0ef56eb00857c0cbb66ee87c577ff527e8/src/core/ddsi/include/dds/ddsi/q_radmin.h#L108
    Bad things will happen if these macros change.
    Some discussions here: https://github.com/eclipse-cyclonedds/cyclonedds/issues/830
*/
fn nn_rdata_payload_offset(rdata: *const nn_rdata) -> usize {
    unsafe { (*rdata).payload_zoff as usize }
}

fn nn_rmsg_payload(rmsg: *const nn_rmsg) -> *const u8 {
    unsafe { rmsg.add(1) as *const u8 }
}

fn nn_rmsg_payload_offset(rmsg: *const nn_rmsg, offset: usize) -> *const u8 {
    unsafe { nn_rmsg_payload(rmsg).add(offset) }
}

/// A reader for a list of scatter gather buffers
struct SGReader<'a> {
    sc_list: Option<Vec<&'a [u8]>>,
    //the current slice that is used
    slice_cursor: usize,
    //the current offset within the slice
    slice_offset: usize,
}

impl<'a> SGReader<'a> {
    pub fn new(sc_list: Vec<&'a [u8]>) -> Self {
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
    use std::ffi::CString;
    use super::*;
    use serde_derive::{Deserialize, Serialize};
    use dds_derive::Topic;
    use crate::{DdsQos, DdsTopic,DdsParticipant, DdsListener};
    
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

        let mut reader = SGReader::new(sc_list);

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
            id : i32,
            x : u32,
            y : u32,
        }
        let foo = Foo { id: 0x12345678,x: 10,y:20};
        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr,vec![0,0,0,0,0x12u8,0x34u8,0x56u8,0x78u8]);
    }
    #[test]
    fn keyhash_simple() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id : i32,
            x : u32,
            #[topic_key]
            s : String,
            y : u32,
        }
        let foo = Foo { id: 0x12345678,x: 10,s: String::from("boo"),  y:20};
        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr,vec![0, 0, 0, 0, 18, 52, 86, 120, 0, 0, 0, 4, 98, 111, 111, 0]);
    }

    #[test]
    fn keyhash_nested() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct NestedFoo {
            name : String,
            val : u64,
            #[topic_key]
            instance: u32,
        }

        impl NestedFoo {
            fn new() -> Self {
                Self {
                    name : "my name".to_owned(),
                    val : 42,
                    instance : 25,
                }
            }
        }

        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id : i32,
            x : u32,
            #[topic_key]
            s : String,
            y : u32,
            #[topic_key]
            inner : NestedFoo,
        }
        let foo = Foo { id: 0x12345678,x: 10,s: String::from("boo"),  y:20, inner: NestedFoo::new()};
        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr,vec![0, 0, 0, 0, 18, 52, 86, 120, 0, 0, 0, 4, 98, 111, 111, 0, 0, 0, 0, 25]);
    }


    #[test]
    fn primitive_array_as_key() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            a : [u8;8],
            b : u32,
            c: String,
        }

        let foo = Foo {
            a : [0,0,0,0,0,0,0,0],
            b : 42,
            c : "foo".to_owned(),
        };

        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr,vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(false, Foo::force_md5_keyhash());
    }

    #[test]
    fn primitive_array_and_string_as_key() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            a : [u8;8],
            b : u32,
            #[topic_key]
            c: String,
        }

        let foo = Foo {
            a : [0,0,0,0,0,0,0,0],
            b : 42,
            c : "foo".to_owned(),
        };

        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr,vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 102, 111, 111, 0]);
        assert_eq!(true, Foo::force_md5_keyhash());
    }


    #[test]
    fn basic() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct NestedFoo {
            name : String,
            val : u64,
            #[topic_key]
            instance: u32,
        }

        impl NestedFoo {
            fn new() -> Self {
                Self {
                    name : "my name".to_owned(),
                    val : 42,
                    instance : 25,
                }
            }
        }

        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id : i32,
            x : u32,
            #[topic_key]
            s : String,
            y : u32,
            #[topic_key]
            inner : NestedFoo,
        }
        let _foo = Foo { id: 0x12345678,x: 10,s: String::from("boo"),  y:20, inner: NestedFoo::new()};
        let t = SerType::<Foo>::new();
        let mut t = SerType::into_sertype(t);
        let tt = &mut t as *mut *mut ddsi_sertype;
        unsafe {
            let p = dds_create_participant (0, std::ptr::null_mut(), std::ptr::null_mut());
            let topic_name = CString::new("topic_name").unwrap();
            let topic = dds_create_topic_sertype(p, topic_name.as_ptr(), tt , std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());

            dds_delete(topic);
        }
    }
}


