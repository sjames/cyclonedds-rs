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
use serde::{Deserialize, __private::ser, de::DeserializeOwned};
use serde_derive::{Deserialize, Serialize};
use std::{ffi::{CStr, c_void}, fmt::format, marker::PhantomData, mem::{self, MaybeUninit}, ops::{Add, Deref}};
use std::io::prelude::*;
use cyclonedds_sys::*;

//const mod_path : String = format!("{}::TopicStruct\0",  module_path!());

#[derive(Default, Deserialize, Serialize, PartialEq)]
struct TopicStruct {
    a: u32,
}

#[repr(C)]
struct SerType<T> {
    sertype: ddsi_sertype,
    _phantom : PhantomData<T>
}

impl <'a,T>SerType<T> {
    fn new() -> Box<SerType<T>> 
    where T: Default + DeserializeOwned {
        Box::<SerType<T>>::new(SerType {
            sertype: {
                let mut sertype = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_sertype_init(
                        sertype.as_mut_ptr(),
                        get_type_name::<T>().as_ptr(),
                        Box::into_raw(create_sertype_ops::<T>()),
                        Box::into_raw(create_serdata_ops::<T>()),
                        true,
                    );
                    sertype.assume_init()
                } 
            },
            _phantom : PhantomData,
        })
    }
}


unsafe extern "C" fn zero_samples<T>(
    sertype: *const ddsi_sertype,
    ptr: *mut std::ffi::c_void,
    len: u64,
) {} // empty implementation

extern "C" fn realloc_samples<T>(
    ptrs: *mut *mut std::ffi::c_void,
    _sertype: *const ddsi_sertype,
    old: *mut std::ffi::c_void,
    old_count: u64,
    new_count: u64,
) {
    let old = unsafe {
        Vec::<T>::from_raw_parts(old as *mut T, old_count as usize, old_count as usize)
    };
    let mut new = Vec::<T>::with_capacity(new_count as usize);

    let copy_count = if new_count < old_count {
        new_count
    } else {
        old_count
    };

    for entry in old {
        new.push(entry);
        if new.len() == copy_count as usize {
            break; // break out if we reached the allocated amount
        }
    }
    // left over samples in the old vector will get freed when it goes out of scope.
    // TODO: must test this.

    let (raw, length, allocated_length) = new.into_raw_parts();
    // if the length and allocated length are not equal, we messed up above.
    assert_eq!(length, allocated_length);
    unsafe {
        *ptrs = raw as *mut std::ffi::c_void;
    }
}

extern "C" fn free_samples<T>(
    _sertype: *const ddsi_sertype,
    ptrs: *mut *mut std::ffi::c_void,
    len: u64,
    op: dds_free_op_t,
) where T: Default {
    let ptrs_v: *mut *mut T = ptrs as *mut *mut T;

    if (op & DDS_FREE_ALL_BIT) != 0 {
        let _samples =
            unsafe { Vec::<T>::from_raw_parts(*ptrs_v, len as usize, len as usize) };
        // all samples will get freed when samples goes out of scope
    } else {
        assert_ne!(op & DDS_FREE_CONTENTS_BIT, 0);
        let mut samples =
            unsafe { Vec::<T>::from_raw_parts(*ptrs_v, len as usize, len as usize) };
        for sample in samples.iter_mut() {
            let _old_sample = std::mem::take(sample);
            //_old_sample goes out of scope and the content is freed. The pointer is replaced with a default constructed sample
        }
    }
}

unsafe extern "C" fn free_sertype<T>(sertype: *mut cyclonedds_sys::ddsi_sertype) {
    ddsi_sertype_fini(sertype) ;

    let _sertype_ops = Box::<ddsi_sertype_ops>::from_raw((&*sertype).ops as *mut ddsi_sertype_ops );
    let _serdata_ops = Box::<ddsi_serdata_ops>::from_raw((&*sertype).serdata_ops as *mut ddsi_serdata_ops );
    // this sertype is always constructed in Rust. During destruction,
    // the Box takes over the pointer and frees it when it goes out
    // of scope.
    let sertype = sertype as *mut SerType<T>;
    Box::<SerType<T>>::from_raw(sertype);
}

 // create ddsi_serdata from a fragchain
 unsafe extern "C" fn serdata_from_fragchain<T> (
    sertype: *const ddsi_sertype,
    kind: u32,
    mut fragchain: *const nn_rdata,
    size: u64,
) -> *mut ddsi_serdata 
where T: DeserializeOwned {
    //make it a reference for convenience
    let off : u32 = 0;
    let size = size as usize;
    let fragchain_ref = &*fragchain;

    let mut serdata = SerData::<T>::new(sertype, kind);
    
    assert_eq!(fragchain_ref.min, 0);
    assert!(fragchain_ref.maxp1 >= off);

    // The scatter gather list
    let mut sg_list = Vec::new();

    while !fragchain.is_null()  {
        let fragchain_ref = &*fragchain;
        if fragchain_ref.maxp1 > off {
            let payload = nn_rmsg_payload_offset(fragchain_ref.rmsg, nn_rdata_payload_offset(fragchain));
            let src = payload.add((off - fragchain_ref.min) as usize);
            let n_bytes = fragchain_ref.maxp1 - off;
            sg_list.push(std::slice::from_raw_parts(src, n_bytes as usize));

        }
        fragchain = fragchain_ref.nextfrag;
    }

    // make a reader out of the sg_list
    let reader = SGReader::new(sg_list);
    if let Ok(decoded) = cdr::deserialize_from::<_,T,_>(reader, Bounded(size as u64) ) {
        let sample = std::sync::Arc::new(decoded);
        //store the deserialized sample in the serdata. We don't need to deserialize again
        serdata.maybe_sample = Some(sample);
    } else {
        println!("Deserialization error!");
        return std::ptr::null_mut();
    }

    // convert into raw pointer and forget about it (for now). Cyclone will take ownership.
    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}


unsafe extern "C" fn serdata_from_iov<T>(
    sertype: *const ddsi_sertype,
    kind: u32,
    niov: u64,
    iov: *const iovec,
    size: u64,
) -> *mut ddsi_serdata 
where T : DeserializeOwned {
    let size = size as usize;
    let niov = niov as usize;

    let mut serdata = SerData::<T>::new(sertype, kind);

    let iovs =
        std::slice::from_raw_parts(iov as *mut *const cyclonedds_sys::iovec, niov as usize);

    let iov_slices: Vec<&[u8]> = iovs
        .iter()
        .map(|iov| {
            let iov = &**iov;
            let slice =
                std::slice::from_raw_parts(iov.iov_base as *const u8, iov.iov_len as usize);
            slice
        })
        .collect();

          // make a reader out of the sg_list
    let reader = SGReader::new(iov_slices);
    if let Ok(decoded) = cdr::deserialize_from::<_,T,_>(reader, Bounded(size as u64) ) {
        let sample = std::sync::Arc::new(decoded);
        //store the deserialized sample in the serdata. We don't need to deserialize again
        serdata.maybe_sample = Some(sample);
    } else {
        println!("Deserialization error!");
        return std::ptr::null_mut();
    }

    // convert into raw pointer and forget about it as ownership is passed into cyclonedds
    let ptr = Box::into_raw(serdata);
    // only we know this ddsi_serdata is really of type SerData
    ptr as *mut ddsi_serdata
}

unsafe extern "C" fn free_serdata<T>(serdata: *mut ddsi_serdata) {
    // the pointer is really a *mut SerData
    let ptr = serdata as *mut SerData<T>;
    let _data = Box::from_raw(ptr);
    // _data goes out of scope and frees the SerData. Nothing more to do here.
}

fn get_type_name<T>() -> std::ffi::CString {
    std::ffi::CString::new(format!("{}", std::any::type_name::<T>()))
        .expect("Unable to create CString for type name")
}

//TODO: check if returning 0 is ok
unsafe extern "C" fn get_size<T>(serdata: *const ddsi_serdata) -> u32 {
    0
}

unsafe extern "C" fn eqkey<T>(serdata_a: *const ddsi_serdata, serdata_b: *const ddsi_serdata) -> bool {
    let a = SerData::<T>::mut_ref_from_serdata(serdata_a);
    let b = SerData::<T>::mut_ref_from_serdata(serdata_b);
    a.key_hash.value == b.key_hash.value
}

fn create_sertype_ops<T>() -> Box<ddsi_sertype_ops> 
where T: Default {
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
where T : DeserializeOwned {
    Box::new(ddsi_serdata_ops {
        eqkey: Some(eqkey::<T>),
        get_size: Some(get_size::<T>),
        from_ser: Some(serdata_from_fragchain::<T>),
        from_ser_iov: Some(serdata_from_iov::<T>),
        from_keyhash: todo!(),
        from_sample: todo!(),
        to_ser: todo!(),
        to_ser_ref: todo!(),
        to_ser_unref: todo!(),
        to_sample: todo!(),
        to_untyped: todo!(),
        untyped_to_sample: todo!(),
        free: Some(free_serdata::<T>),
        print: todo!(),
        get_keyhash: todo!(),
    })
}

    // not sure what this needs to do. The C++ implementation at
    // https://github.com/eclipse-cyclonedds/cyclonedds-cxx/blob/templated-streaming/src/ddscxx/include/org/eclipse/cyclonedds/topic/datatopic.hpp
    // just returns 0
    unsafe extern "C" fn hash<T>(_acmn: *const ddsi_sertype) -> u32 {
        0
    }

    unsafe extern "C" fn equal<T>(acmn: *const ddsi_sertype, bcmn: *const ddsi_sertype) -> bool {

        let acmn = CStr::from_ptr((*acmn).type_name as *mut i8);
        let bcmn = CStr::from_ptr((*bcmn).type_name as *mut i8);
        acmn == bcmn
    }



/// A representation for the serialized data.
#[repr(C)]
struct SerData<T> {
    serdata: ddsi_serdata,
    maybe_sample : Option<std::sync::Arc<T>>,
    key_hash : ddsi_keyhash,
}

impl <'a,T>SerData<T> {
    fn new(sertype: *const ddsi_sertype, kind: u32) -> Box<SerData<T>> {
        Box::<SerData<T>>::new(SerData {
            serdata: {
                let mut data = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_serdata_init(data.as_mut_ptr(), sertype, kind);
                    data.assume_init()
                }
            },
            maybe_sample : None,
            key_hash : ddsi_keyhash::default()
        })
    }

    fn mut_ref_from_serdata(serdata: *const ddsi_serdata) -> &'a mut Self {
        let ptr = serdata as *mut SerData<T>;
        unsafe {&mut*ptr}
    }
}

/*  These functions are created from the macros in 
    https://github.com/eclipse-cyclonedds/cyclonedds/blob/f879dc0ef56eb00857c0cbb66ee87c577ff527e8/src/core/ddsi/include/dds/ddsi/q_radmin.h#L108
    Bad things will happen if these macros change. 
    Some discussions here: https://github.com/eclipse-cyclonedds/cyclonedds/issues/830
*/
fn nn_rdata_payload_offset(rdata: *const nn_rdata) -> usize {
    unsafe {(&*rdata).payload_zoff as usize} 
}

fn nn_rmsg_payload(rmsg : *const nn_rmsg) -> * const u8{
    unsafe {
        rmsg.add(1) as *const u8
    }
}

fn nn_rmsg_payload_offset(rmsg : *const nn_rmsg, offset: usize) -> * const u8{
    unsafe {
        nn_rmsg_payload(rmsg).add(offset)
    }
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
    use super::*;
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

        let mut buf = vec![0,0,0,0,0];
        if  let Ok(n) = reader.read(&mut buf) {
            assert_eq!(&buf[..n],vec![1, 2, 3, 4, 5]);
        } else {
            panic!("should not panic");
        }
        if  let Ok(n) = reader.read(&mut buf) {
            assert_eq!(&buf[..n],vec![6]);
        } else {
            panic!("should not panic");
        }
        
    }

    #[test]
    fn basic() {
        let t =  SerType::<TopicStruct>::new();
    }
}

