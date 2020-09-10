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

use crate::{DdsParticipant, Entity};
pub use cyclonedds_sys::{DDSError, DdsDomainId, DdsEntity};
use std::convert::From;
use std::marker::PhantomData;

pub struct DdsWaitset<T>(DdsEntity, PhantomData<*const T>);

impl<T> DdsWaitset<T> {
    pub fn create(participant: &DdsParticipant) -> Result<Self, DDSError> {
        unsafe {
            let p = cyclonedds_sys::dds_create_waitset(participant.entity().entity());
            if p > 0 {
                Ok(DdsWaitset(DdsEntity::new(p), PhantomData))
            } else {
                Err(DDSError::from(p))
            }
        }
    }

    pub fn attach(&mut self, entity: &DdsEntity, x: Box<T>) -> Result<(), DDSError> {
        Ok(())
    }
    pub fn detach(&mut self, entity: &DdsEntity) -> Result<(), DDSError> {
        Ok(())
    }
    pub fn set_trigger(&mut self) -> Result<(), DDSError> {
        Ok(())
    }
    pub fn wait(&mut self) -> Result<Vec<Box<T>>, DDSError> {
        Ok(Vec::new())
    }
}

impl<T> Drop for DdsWaitset<T> {
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0.entity()).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete DdsWaitset: {}", ret);
            } else {
            }
        }
    }
}



