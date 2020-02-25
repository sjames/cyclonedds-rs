use crate::error::DDSError;
//use cyclonedds_sys::*;
use std::convert::From;

pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t};

use crate::{dds_listener::DdsListener, dds_qos::DdsQos};

#[derive(Clone)]
pub struct DdsParticipant(dds_entity_t);

impl DdsParticipant {
    pub fn create(
        maybe_domain: Option<dds_domainid_t>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let p = cyclonedds_sys::dds_create_participant(
                maybe_domain.unwrap_or(0xFFFF_FFFF),
                maybe_qos.map_or(std::ptr::null(), |d| d.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );
            if p > 0 {
                Ok(DdsParticipant(p))
            } else {
                Err(DDSError::from(p))
            }
        }
    }
}

impl From<DdsParticipant> for dds_entity_t {
    fn from(domain: DdsParticipant) -> Self {
        domain.0
    }
}

#[cfg(test)]
mod dds_participant_tests {
    use super::*;

    #[test]
    fn test_create() {
        let mut qos = DdsQos::create().unwrap();
        qos.set_lifespan(1000);
        let _par = DdsParticipant::create(None, Some(qos), None);
    }
}
