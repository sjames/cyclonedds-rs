use cyclonedds_sys::*;
use std::convert::From;
use std::os::raw::c_void;

pub use cyclonedds_sys::{DDSBox, DDSGenType, DdsDomainId, DdsEntity};

pub use either::Either;
use std::marker::PhantomData;

use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos,
    dds_subscriber::DdsSubscriber, dds_topic::DdsTopic,
};

pub struct DdsReader<T: Sized + DDSGenType> {
    entity: dds_entity_t,
    listener: Option<DdsListener>,
    _phantom: PhantomData<*const T>,
    // The callback closures that can be attached to a reader
}

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
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsReader {
                    entity: w,
                    listener: maybe_listener,
                    _phantom: PhantomData,
                })
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn set_listener(&mut self, listener: DdsListener) -> Result<(), DDSError> {
        unsafe {
            let refl = &listener;
            let rc = dds_set_listener(self.entity, refl.into());
            if rc == 0 {
                self.listener = Some(listener);
                Ok(())
            } else {
                Err(DDSError::from(rc))
            }
        }
    }

    /// Read a buffer given a dds_entity_t.  This is useful when you want to read data
    /// within a closure.
    pub fn read_from_entity(entity: DdsEntity) -> Result<DDSBox<T>, DDSError> {
        unsafe {
            let mut info = cyclonedds_sys::dds_sample_info::default();
            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_read(entity, voidpp, &mut info as *mut _, 1, 1);

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

    pub fn read(&self) -> Result<DDSBox<T>, DDSError> {
        unsafe {
            let mut info: dds_sample_info = dds_sample_info::default();
            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_read(self.entity, voidpp, &mut info as *mut _, 1, 1);

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

impl<T> From<DdsReader<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(reader: DdsReader<T>) -> Self {
        reader.entity
    }
}

impl<T> From<&DdsReader<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(reader: &DdsReader<T>) -> Self {
        reader.entity
    }
}
