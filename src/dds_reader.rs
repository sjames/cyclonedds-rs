use crate::error::DDSError;
use cyclonedds_sys::*;
use std::convert::From;
use std::os::raw::c_void;

pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t, dds_sample_info, DDSBox, DDSGenType};
pub use either::Either;
use std::marker::PhantomData;

use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos,
    dds_subscriber::DdsSubscriber, dds_topic::DdsTopic,
};

pub struct DdsReader<T: Sized + DDSGenType>(dds_entity_t, PhantomData<*const T>);

impl<T> DdsReader<T>
where
    T: Sized + DDSGenType,
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

    pub fn read(&mut self) -> Result<DDSBox<T>, DDSError> {
        unsafe {
            let mut info: dds_sample_info = dds_sample_info::default();

            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_read(self.0, voidpp, &mut info as *mut _, 1, 1);

            if ret >= 0 {
                if !voidp.is_null() && info.valid_data {
                    let buf = DDSBox::<T>::new_from_cyclone_allocated_struct(voidp as *mut T);
                    Ok(buf)
                } else {
                    Err(DDSError::OutOfResources)
                }
            } else {
                Err(DDSError::from(ret))
            }
        }
    }
}
