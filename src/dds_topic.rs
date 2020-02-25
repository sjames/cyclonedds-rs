use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos, error::DDSError,
};

use std::marker::PhantomData;

use std::convert::From;

use cyclonedds_sys::dds_create_topic;
pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t, dds_topic_descriptor_t};

pub struct DdsTopic<T: Sized>(dds_entity_t, PhantomData<*const T>);

impl<T> DdsTopic<T>
where
    T: std::marker::Sized,
{
    pub fn create(
        participant: DdsParticipant,
        descriptor: *const dds_topic_descriptor_t,
        name: &str,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let strname = name.as_ptr();
            let topic = dds_create_topic(
                participant.into(),
                descriptor,
                strname as *const i8,
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );

            if topic > 0 {
                Ok(DdsTopic(topic, PhantomData))
            } else {
                Err(DDSError::from(topic))
            }
        }
    }
}

impl<T> From<DdsTopic<T>> for dds_entity_t {
    fn from(domain: DdsTopic<T>) -> Self {
        domain.0
    }
}
