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

use cyclonedds_sys::*;
use std::convert::From;

pub use cyclonedds_sys::{DDSBox, DdsEntity};
use std::marker::PhantomData;

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsWritable, Entity};

pub struct DdsWriter<'a, T: Sized + DDSGenType>(
    DdsEntity,
    Option<DdsListener<'a>>,
    Option<&'a DdsQos>,
    PhantomData<*const T>,
);

impl<'a, T> DdsWriter<'a, T>
where
    T: Sized + DDSGenType,
{
    pub fn create(
        entity: &dyn DdsWritable,
        topic: &DdsTopic<T>,
        maybe_qos: Option<&'a DdsQos>,
        maybe_listener: Option<DdsListener<'a>>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_writer(
                entity.entity().entity(),
                topic.entity().entity(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsWriter(
                    DdsEntity::new(w),
                    maybe_listener,
                    maybe_qos,
                    PhantomData,
                ))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn write_to_entity(entity: DdsEntity, msg: &DDSBox<T>) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_write(entity.entity(), msg.get_raw_mut_ptr());
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn write(&mut self, msg: &T) -> Result<(), DDSError> {
        unsafe {
            let ret = dds_write(self.0.entity(), msg as *const T as *const std::ffi::c_void);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn set_listener(&mut self, listener: DdsListener<'a>) -> Result<(), DDSError> {
        unsafe {
            let refl = &listener;
            let rc = dds_set_listener(self.0.entity(), refl.into());
            if rc == 0 {
                self.1 = Some(listener);
                Ok(())
            } else {
                Err(DDSError::from(rc))
            }
        }
    }
}

impl<'a, T> Entity for DdsWriter<'a, T>
where
    T: std::marker::Sized + DDSGenType,
{
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

impl<'a, T> Drop for DdsWriter<'a, T>
where
    T: std::marker::Sized + DDSGenType,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0.entity()).into();
            if DDSError::DdsOk != ret && DDSError::AlreadyDeleted != ret {
                panic!("cannot delete Writer: {}", ret);
            }
        }
    }
}
