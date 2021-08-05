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

use crate::{DdsListener, DdsParticipant, DdsQos, DdsReadable};
pub use cyclonedds_sys::{DDSError, DdsDomainId, DdsEntity};
use std::{convert::From};

#[derive(Clone)]
pub struct DdsSubscriber(DdsEntity);

impl<'a> DdsSubscriber {
    pub fn create(
        participant: &DdsParticipant,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let p = cyclonedds_sys::dds_create_subscriber(
                participant.entity().entity(),
                maybe_qos.map_or(std::ptr::null(), |d| d.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );
            if p > 0 {
                Ok(DdsSubscriber(DdsEntity::new(p)))
            } else {
                Err(DDSError::from(p))
            }
        }
    }
}


impl<'a> DdsReadable for DdsSubscriber {
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}
