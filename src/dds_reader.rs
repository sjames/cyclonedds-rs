use crate::error::DDSError;
use cyclonedds_sys::*;
use std::convert::From;
use std::os::raw::c_void;
use std::mem::MaybeUninit;

pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t,dds_sample_info, DdsAllocator};
pub use either::Either;
use std::marker::PhantomData;

use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_subscriber::DdsSubscriber,
    dds_qos::DdsQos, dds_topic::DdsTopic,
};

pub struct DdsReader<T: Sized + DdsAllocator>(dds_entity_t, PhantomData<*const T>);

impl<T> DdsReader<T>
where
    T: Sized + DdsAllocator
{
    pub fn create(
        entity: Either<&DdsParticipant, &DdsSubscriber>,
        topic: &DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_reader(
                entity.either(|l| l.into(), |r| r.into()),
                topic.into(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsReader(w, PhantomData))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

/*

        pub fn read(&mut self) -> Result<T, DDSError> {
        unsafe {
             let mut info: dds_sample_info = dds_sample_info::default();
             let mut msg : T = T::new();
             
            // Read more: https://stackoverflow.com/questions/24191249/working-with-c-void-in-an-ffi
            let voidp: *mut *mut c_void = msg as *mut _ as *mut *mut c_void;
            let ret = dds_read(self.0, voidp, &mut info as *mut _ ,1, 1);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
        
    }*/
}