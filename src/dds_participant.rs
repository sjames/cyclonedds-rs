/*
    Copyright 2020 Sojan James

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

use std::convert::From;

pub use cyclonedds_sys::{DDSError, DdsDomainId, DdsEntity};

use crate::{DdsReadable, DdsWritable, Entity, dds_listener::DdsListener, dds_qos::DdsQos};


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
                Ok(DdsParticipant(DdsEntity::new(p)))
            } else {
                Err(DDSError::from(p))
            }
        }
    }
}

/* 
impl Drop for DdsParticipant {
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0.entity()).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete participant: {}", ret);
            } else {
                //println!("Participant dropped");
            }
        }
    }
}
*/

impl DdsWritable for DdsParticipant {
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

impl DdsReadable for DdsParticipant {
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

impl Entity for DdsParticipant {
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

#[cfg(test)]
mod dds_participant_tests {
    use super::*;

    #[test]
    fn test_create() {
        let qos = DdsQos::create().unwrap()
        .set_lifespan(std::time::Duration::from_nanos(1000));
        let _par = DdsParticipant::create(None, Some(qos), None);
    }
}
