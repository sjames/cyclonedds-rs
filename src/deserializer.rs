// Rust deserializer for CycloneDDS.
// See discussion at https://github.com/eclipse-cyclonedds/cyclonedds/issues/830

use cdr::{CdrBe, Infinite};
use serde::__private::ser;
use serde_derive::{Deserialize, Serialize};
use std::{
    ffi::CStr,
    fmt::format,
    mem::{self, MaybeUninit},
    ops::Deref,
};

use cyclonedds_sys::*;

//const mod_path : String = format!("{}::TopicStruct\0",  module_path!());

#[derive(Default, Deserialize, Serialize, PartialEq)]
struct TopicStruct {
    a: u32,
}

struct SerType {
    sertype: ddsi_sertype,
}

impl SerType {
    fn new<T>() -> Box<SerType> 
    where T: Topic {
        Box::<SerType>::new(SerType {
            sertype: {
                let mut sertype = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_sertype_init(
                        sertype.as_mut_ptr(),
                        T::get_type_name().as_ptr(),
                        Box::into_raw(T::create_sertype_ops()),
                        Box::into_raw(T::create_serdata_ops()),
                        true,
                    );
                    sertype.assume_init()
                } 
            },
        })
    }
}

pub trait Topic {
    fn get_type_name() -> std::ffi::CString;
    fn create_sertype_ops() -> Box<ddsi_sertype_ops>;
    fn create_serdata_ops() -> Box<ddsi_serdata_ops>;
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

unsafe extern "C" fn free_sertype(sertype: *mut cyclonedds_sys::ddsi_sertype) {
    ddsi_sertype_fini(sertype) ;

    let _sertype_ops = Box::<ddsi_sertype_ops>::from_raw((&*sertype).ops as *mut ddsi_sertype_ops );
    let _serdata_ops = Box::<ddsi_serdata_ops>::from_raw((&*sertype).serdata_ops as *mut ddsi_serdata_ops );
    // this sertype is always constructed in Rust. During destruction,
    // the Box takes over the pointer and frees it when it goes out
    // of scope.
    let sertype = sertype as *mut SerType;
    Box::<SerType>::from_raw(sertype);
}

// test
impl Topic for TopicStruct {
    fn get_type_name() -> std::ffi::CString {
        std::ffi::CString::new(format!("{}::TopicStruct", module_path!()))
            .expect("Unable to create CString for type name")
    }
    fn create_sertype_ops() -> Box<ddsi_sertype_ops> {
        Box::new(ddsi_sertype_ops {
            version: Some(ddsi_sertype_v0),
            arg: std::ptr::null_mut(),
            free: Some(free_sertype),
            zero_samples: Some(zero_samples::<Self>),
            realloc_samples: Some(realloc_samples::<Self>),
            free_samples: Some(free_samples::<Self>),
            equal: Some(Self::equal),
            hash: Some(Self::hash),
            typeid_hash: None,
            serialized_size: None,
            serialize: None,
            deserialize: None,
            assignable_from: None,
        })
    }
    fn create_serdata_ops() -> Box<ddsi_serdata_ops> {
        Box::new(ddsi_serdata_ops {
            eqkey: todo!(),
            get_size: todo!(),
            from_ser: Some(Self::serdata_from_fragchain),
            from_ser_iov: Some(Self::serdata_from_iov),
            from_keyhash: todo!(),
            from_sample: todo!(),
            to_ser: todo!(),
            to_ser_ref: todo!(),
            to_ser_unref: todo!(),
            to_sample: todo!(),
            to_untyped: todo!(),
            untyped_to_sample: todo!(),
            free: Some(Self::free_serdata),
            print: todo!(),
            get_keyhash: todo!(),
        })
    }

}

impl TopicStruct {
    pub fn create_type() -> *mut ddsi_sertype {
        let sertype = SerType::new::<TopicStruct>();
        let ptr = Box::into_raw(sertype);
        ptr as *mut ddsi_sertype
     }


    // not sure what this needs to do. The C++ implementation at
    // https://github.com/eclipse-cyclonedds/cyclonedds-cxx/blob/templated-streaming/src/ddscxx/include/org/eclipse/cyclonedds/topic/datatopic.hpp
    // just returns 0
    unsafe extern "C" fn hash(_acmn: *const ddsi_sertype) -> u32 {
        0
    }

    unsafe extern "C" fn equal(acmn: *const ddsi_sertype, bcmn: *const ddsi_sertype) -> bool {

        let acmn = CStr::from_ptr((*acmn).type_name as *mut i8);
        let bcmn = CStr::from_ptr((*bcmn).type_name as *mut i8);
        if acmn == bcmn {
            true
        } else {
            false
        }

    }

    unsafe extern "C" fn serdata_from_iov(
        sertype: *const ddsi_sertype,
        kind: u32,
        niov: u64,
        iov: *const iovec,
        size: u64,
    ) -> *mut ddsi_serdata {
        let size = size as usize;
        let niov = niov as usize;

        let serdata = SerData::new(sertype, kind, size);

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

        // convert into raw pointer and forget about it as ownership is passed into cyclonedds
        let ptr = Box::into_raw(serdata);
        // only we know this ddsi_serdata is really of type SerData
        ptr as *mut ddsi_serdata
    }

    // create ddsi_serdata from a fragchain
    unsafe extern "C" fn serdata_from_fragchain(
        sertype: *const ddsi_sertype,
        kind: u32,
        fragchain: *const nn_rdata,
        size: u64,
    ) -> *mut ddsi_serdata {
        //make it a reference for convenience
        let size = size as usize;
        let fragchain = &*fragchain;
        let serdata = SerData::new(sertype, kind, size);

        assert_eq!(fragchain.min, 0);
        assert!(fragchain.maxp1 >= 0);

        // convert into raw pointer and forget about it
        let ptr = Box::into_raw(serdata);
        // only we know this ddsi_serdata is really of type SerData
        ptr as *mut ddsi_serdata
    }

    unsafe extern "C" fn free_serdata(serdata: *mut ddsi_serdata) {
        // the pointer is really a *mut SerData
        let ptr = serdata as *mut SerData;
        let _data = Box::from_raw(ptr);
        // _data goes out of scope and frees the SerData. Nothing more to do here.
    }
}

/*
void* data;
    size_t data_size;
    void* key;
    size_t key_size;
    ddsi_keyhash_t hash;
    bool key_populated;
    bool data_is_key;
 */

#[repr(C)]
struct SerData {
    serdata: ddsi_serdata,
    data_size: usize,
    key_size: usize,
    key_populated: bool,
    data_is_key: bool,
}

impl SerData {
    fn new(sertype: *const ddsi_sertype, kind: u32, data_size: usize) -> Box<SerData> {
        Box::<SerData>::new(SerData {
            serdata: {
                let mut data = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_serdata_init(data.as_mut_ptr(), sertype, kind);
                    data.assume_init()
                }
            },
            data_size,
            key_size: 0,
            key_populated: false,
            data_is_key: false,
        })
    }
}

/*
struct FragChain<'a>{chain: &'a nn_rdata, offset:u32, current:&'a nn_rdata}

impl <'a>Iterator for FragChain<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<Self::Item> {

        if self.chain.maxp1 > self.offset {
            let payload = get_payload_with_offset(self.current.rmsg, o)
        }

        // Since there's no endpoint to a Fibonacci sequence, the `Iterator`
        // will never return `None`, and `Some` is always returned.
        //Some(self.curr)
        None
    }
}

// Warning: These functions are derived from the macros
// in dds/ddsi/q_radmin.h.
//#define NN_RMSG_PAYLOAD(m) ((unsigned char *) (m + 1))
//#define NN_RMSG_PAYLOADOFF(m, o) (NN_RMSG_PAYLOAD (m) + (o))
// All this will break if these macros change.
// TODO: figure out a way to check this at compile time?
fn get_payload(msg:&[u8]) -> &[u8] {
    &msg[1..]
}

fn get_payload_with_offset(msg:&[u8], o: usize) -> &[u8] {
    &get_payload(msg)[o..]
}

*/

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn basic() {
        let t = TopicStruct::create_type();
    }
}

