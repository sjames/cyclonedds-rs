use crate::error::DDSError;
use cyclonedds_sys::*;
use std::convert::From;

pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t};
pub use either::Either;
use std::marker::PhantomData;

use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_publisher::DdsPublisher,
    dds_qos::DdsQos, dds_topic::DdsTopic,
};

pub struct DdsWriter<T: Sized + DDSGenType>(dds_entity_t, PhantomData<*const T>);

impl<T> DdsWriter<T>
where
    T: Sized + DDSGenType,
{
    pub fn create(
        entity: Either<&DdsParticipant, &DdsPublisher>,
        topic: &DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_writer(
                entity.either(|l| l.into(), |r| r.into()),
                topic.into(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsWriter(w, PhantomData))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn write(&mut self, msg: &T) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_write(self.0, msg.get_raw_ptr());
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }
}

impl<T> From<DdsWriter<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(writer: DdsWriter<T>) -> Self {
        writer.0
    }
}

impl<T> From<&DdsWriter<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(writer: &DdsWriter<T>) -> Self {
        writer.0
    }
}
