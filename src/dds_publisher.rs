use crate::error::DDSError;
use cyclonedds_sys::*;
use std::convert::From;
use std::ffi::CString;

pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t};
pub use either::Either;

use crate::{dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos};

pub struct DdsPublisher(dds_entity_t);

impl From<DdsPublisher> for dds_entity_t {
    fn from(publisher: DdsPublisher) -> Self {
        publisher.0
    }
}
impl From<&DdsPublisher> for dds_entity_t {
    fn from(publisher: &DdsPublisher) -> Self {
        publisher.0
    }
}