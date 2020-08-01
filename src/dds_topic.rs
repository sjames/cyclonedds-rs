use crate::{dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos};

use std::convert::From;
use std::ffi::CString;
use std::marker::PhantomData;

pub use cyclonedds_sys::{DDSError, DDSGenType, DdsEntity};

pub struct DdsTopic<T: Sized + DDSGenType>(DdsEntity, PhantomData<*const T>);

impl<T> DdsTopic<T>
where
    T: std::marker::Sized + DDSGenType,
{
    pub fn create(
        participant: &DdsParticipant,
        name: &str,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let strname = CString::new(name).expect("CString::new failed");
            let topic = cyclonedds_sys::dds_create_topic(
                participant.into(),
                T::get_descriptor(),
                strname.as_ptr(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );

            if topic >= 0 {
                Ok(DdsTopic(topic, PhantomData))
            } else {
                Err(DDSError::from(topic))
            }
        }
    }
}

impl<T> From<DdsTopic<T>> for DdsEntity
where
    T: std::marker::Sized + DDSGenType,
{
    fn from(domain: DdsTopic<T>) -> Self {
        domain.0
    }
}

impl<T> From<&DdsTopic<T>> for DdsEntity
where
    T: std::marker::Sized + DDSGenType,
{
    fn from(domain: &DdsTopic<T>) -> Self {
        domain.0
    }
}

impl<T> Drop for DdsTopic<T>
where
    T: std::marker::Sized + DDSGenType,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete Topic: {}", ret);
            }
        }
    }
}
