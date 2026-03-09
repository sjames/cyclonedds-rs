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

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, DdsReadable, DdsWritable, Entity};
pub use cyclonedds_sys::{DDSError, DdsDomainId, DdsEntity};
use std::convert::From;
use tracing::trace;

/// Builder struct for a Participant.
/// # Example
/// ```text
/// use cyclonedds_rs::{DdsListener, ParticipantBuilder};
/// let listener = DdsListener::new()
///   .on_subscription_matched(|a,b| {
///     println!("Subscription matched!");
/// }).on_publication_matched(|a,b|{
///     println!("Publication matched");
/// }).
/// hook();
/// let participant = ParticipantBuilder::new()
///         .with_listener(listener)
///         .create()
///         .expect("Unable to create participant");
///
///```
///
pub struct ParticipantBuilder {
    maybe_domain: Option<DdsDomainId>,
    maybe_qos: Option<DdsQos>,
    maybe_listener: Option<DdsListener>,
}

impl Default for ParticipantBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ParticipantBuilder {
    pub fn new() -> Self {
        ParticipantBuilder {
            maybe_domain: None,
            maybe_qos: None,
            maybe_listener: None,
        }
    }

    pub fn with_domain(mut self, domain: DdsDomainId) -> Self {
        self.maybe_domain = Some(domain);
        self
    }

    pub fn with_qos(mut self, qos: DdsQos) -> Self {
        self.maybe_qos = Some(qos);
        self
    }

    pub fn with_listener(mut self, listener: DdsListener) -> Self {
        self.maybe_listener = Some(listener);
        self
    }

    pub fn create(self) -> Result<DdsParticipant, DDSError> {
        DdsParticipant::create(self.maybe_domain, self.maybe_qos, self.maybe_listener)
    }
}

#[allow(dead_code)]
pub struct DdsParticipant(DdsEntity, Option<DdsListener>);

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
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );
            if p > 0 {
                let p = DdsParticipant(DdsEntity::new(p), maybe_listener);
                trace!("Created participant with guid: {}", p.guid());
                Ok(p)
            } else {
                Err(DDSError::from(p))
            }
        }
    }

    pub fn guid(&self) -> uuid::Uuid {
        let mut guid = cyclonedds_sys::dds_guid_t::default();
        let ret = unsafe { cyclonedds_sys::dds_get_guid(self.0.entity(), &mut guid) };
        if ret >= 0 {
            parse_guid(&guid)
        } else {
            uuid::Uuid::nil()
        }
    }
}

impl Drop for DdsParticipant {
    fn drop(&mut self) {
        trace!("Dropping participant with guid: {}", self.guid());
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0.entity()).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete participant: {}", ret);
            }
        }
    }
}

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

pub(crate) fn parse_guid(guid_t: &cyclonedds_sys::dds_guid_t) -> uuid::Uuid {
    uuid::Uuid::from_bytes(guid_t.v)
}

#[cfg(test)]
mod dds_participant_tests {
    use super::*;

    #[test]
    fn test_create() {
        let mut qos = DdsQos::create().unwrap();
        qos.set_lifespan(std::time::Duration::from_nanos(1000));
        let _par = DdsParticipant::create(None, Some(qos), None);
    }
}
