use cyclonedds_sys::*;
use std::convert::From;

pub use cyclonedds_sys::{DDSBox, DdsEntity};
use std::marker::PhantomData;

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsWritable};

pub struct DdsWriter<T: Sized + DDSGenType>(DdsEntity, Option<DdsListener>, PhantomData<*const T>);

impl<T> DdsWriter<T>
where
    T: Sized + DDSGenType,
{
    pub fn create(
        entity: &dyn DdsWritable,
        topic: &DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_writer(
                entity.entity(),
                topic.into(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsWriter(w, maybe_listener, PhantomData))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn write_to_entity(entity: DdsEntity, msg: &DDSBox<T>) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_write(entity, msg.get_raw_mut_ptr());
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn write(&mut self, msg: &T) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_write(self.0, msg as *const T as *const std::ffi::c_void);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn set_listener(&mut self, listener: DdsListener) -> Result<(), DDSError> {
        unsafe {
            let refl = &listener;
            let rc = dds_set_listener(self.0, refl.into());
            if rc == 0 {
                self.1 = Some(listener);
                Ok(())
            } else {
                Err(DDSError::from(rc))
            }
        }
    }

    pub fn entity(&self) -> DdsEntity {
        self.into()
    }
}

impl<T> From<DdsWriter<T>> for DdsEntity
where
    T: Sized + DDSGenType,
{
    fn from(writer: DdsWriter<T>) -> Self {
        writer.0
    }
}

impl<T> From<&DdsWriter<T>> for DdsEntity
where
    T: Sized + DDSGenType,
{
    fn from(writer: &DdsWriter<T>) -> Self {
        writer.0
    }
}

impl<T> Drop for DdsWriter<T>
where
    T: std::marker::Sized + DDSGenType,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0).into();
            if DDSError::DdsOk != ret && DDSError::AlreadyDeleted != ret {
                panic!("cannot delete Writer: {}", ret);
            }
        }
    }
}
