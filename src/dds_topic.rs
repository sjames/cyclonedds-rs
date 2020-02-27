use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos, error::DDSError,
};

use std::marker::PhantomData;

use std::convert::From;
use std::ffi::{CString};

use cyclonedds_sys::dds_create_topic;
pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t, dds_topic_descriptor_t};

pub struct DdsTopic<T: Sized>(dds_entity_t, PhantomData<*const T>);

impl<T> DdsTopic<T>
where
    T: std::marker::Sized,
{
    pub fn create(
        participant: &DdsParticipant,
        descriptor: &'static dds_topic_descriptor_t,
        name: &str,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let strname =  CString::new(name).expect("CString::new failed");
            let topic = dds_create_topic(
                participant.into(),
                descriptor,
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

impl<T> From<DdsTopic<T>> for dds_entity_t {
    fn from(domain: DdsTopic<T>) -> Self {
        domain.0
    }
}

impl<T> From<&DdsTopic<T>> for dds_entity_t {
    fn from(domain: &DdsTopic<T>) -> Self {
        domain.0
    }
}

#[macro_export]
macro_rules! type_descriptor {
    ($ddstype:ident) => {
        unsafe {
             paste::expr!{ &[<$ddstype _desc>] as &'static dds_topic_descriptor_t}
        }
    };
}

/// Create a topic given the topic type and the topic name.
/// Example:
///     let topic = create_topic!(participant,HelloWorldData_Msg,"HelloWorldData_Msg", None, None).unwrap();
#[macro_export]
macro_rules! create_topic {
    ($participant:ident,$ddstype:ident,$topic_name:expr,$maybe_qos:expr,$maybe_listener:expr) => {
        DdsTopic::<$ddstype>::create(&$participant, type_descriptor!($ddstype) , $topic_name, $maybe_qos, $maybe_listener)
    }
}