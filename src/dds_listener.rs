use crate::error::DDSError;
use cyclonedds_sys::{dds_listener_t};
use std::convert::From;

pub struct DdsListener(*mut dds_listener_t);

impl From<DdsListener> for *const dds_listener_t {
    fn from(listener: DdsListener) -> Self {
        listener.0
    }
}
