// Rust deserializer for CycloneDDS.
// See discussion at https://github.com/eclipse-cyclonedds/cyclonedds/issues/830

use std::{fmt::format, mem::{self, MaybeUninit}};

use cyclonedds_sys::*;

//const mod_path : String = format!("{}::TopicStruct\0",  module_path!());

struct TopicStruct {
    a : u32,
}

struct DdsiSerType(ddsi_sertype);


impl TopicStruct {
    pub fn create_type() -> ddsi_sertype {
        let mut maybe_sertype : MaybeUninit<cyclonedds_sys::ddsi_sertype> = MaybeUninit::uninit();
        let sertype_ops = Self::create_sertype_ops();
        let serdata_ops = Self::create_serdata_ops();

        let nokey = true;
        let name = std::ffi::CString::new("TopicStruct").unwrap();

        unsafe {
            cyclonedds_sys::ddsi_sertype_init(
                maybe_sertype.as_mut_ptr(),
                Self::get_type_name().as_ptr(),
                &sertype_ops,
                &serdata_ops,
                nokey,
                );
                maybe_sertype.assume_init()
        }
    }


    fn get_type_name() -> std::ffi::CString {
        std::ffi::CString::new(format!("{}::TopicStruct",module_path!())).expect("Unable to create CString for type name")
    }

    fn create_sertype_ops() -> ddsi_sertype_ops {
        ddsi_sertype_ops {
            version:Some(ddsi_sertype_v0),
            arg: std::ptr::null_mut(),
            free: Some(Self::free),
            zero_samples: Some(Self::zero_samples),
            realloc_samples: Some(Self::realloc_samples),
            free_samples: todo!(),
            equal: todo!(),
            hash: todo!(),
            typeid_hash: None,
            serialized_size: None,
            serialize: None,
            deserialize: None,
            assignable_from: None,
        }
    }

    fn create_serdata_ops() -> ddsi_serdata_ops {
        todo!()
    }

    extern "C" fn free(sertype: *mut cyclonedds_sys::ddsi_sertype) {
        unsafe {ddsi_sertype_fini(sertype)};
        // this sertype is always constructed in Rust. During destruction,
        // the Box takes over the pointer and frees it when it goes out
        // of scope.
        unsafe {
            Box::<ddsi_sertype>::from_raw(sertype);
        }
    }

    // The C++ serializer does not implement this function. So leaving as is for now.
    extern "C" fn zero_samples(sertype: *const ddsi_sertype, ptr: *mut std::ffi::c_void, len: usize) {
       
    }

    extern "C" fn realloc_samples(mut ptrs : *mut *mut std::ffi::c_void, _sertype: *const ddsi_sertype, old: *mut std::ffi::c_void, old_count: usize, new_count: usize) {
        let old = unsafe {Vec::<Self>::from_raw_parts(old as *mut TopicStruct, old_count, old_count)};
        let mut new = Vec::<Self>::with_capacity(new_count);

        let copy_count = if new_count < old_count { new_count} else { old_count};

        for entry in old {
            new.push(entry);
            if new.len() == copy_count {
                break; // break out if we reached the allocated amount
            } 
        }

        // left over samples in the old vector will get freed when it goes out of scope.
        // TODO: must test this.
        
        let (raw, length, allocated_length) = new.into_raw_parts();
        // if the length and allocated length are not equal, we messed up above.
        assert_eq!(length, allocated_length);
        unsafe {
            *ptrs = raw as *mut std::ffi::c_void ;
        }
    }

}


fn test() {



    
}