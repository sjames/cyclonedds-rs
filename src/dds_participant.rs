use std::convert::From;

pub use cyclonedds_sys::{DDSError, DdsDomainId, DdsEntity};

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, DdsReadable, DdsWritable};

#[derive(Clone)]
pub struct DdsParticipant(DdsEntity);

impl DdsParticipant {
    pub fn create(
        maybe_domain: Option<DdsDomainId>,
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

impl From<DdsParticipant> for DdsEntity {
    fn from(domain: DdsParticipant) -> Self {
        domain.0
    }
}

impl From<&DdsParticipant> for DdsEntity {
    fn from(domain: &DdsParticipant) -> Self {
        domain.0
    }
}

impl Drop for DdsParticipant {
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete participant: {}", ret);
            } else {
                //println!("Participant dropped");
            }
        }
    }
}

impl DdsWritable for DdsParticipant {
    fn entity(&self) -> DdsEntity {
        self.into()
    }
}

impl DdsReadable for DdsParticipant {
    fn entity(&self) -> DdsEntity {
        self.into()
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
