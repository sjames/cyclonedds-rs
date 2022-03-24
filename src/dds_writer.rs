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
use std::ffi::c_void;

pub use cyclonedds_sys::{ DdsEntity};
use std::marker::PhantomData;

use crate::SampleBuffer;
use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsWritable, Entity};
use crate::serdes::{Sample, TopicType};

pub struct WriterBuilder<T: TopicType> {
    maybe_qos: Option<DdsQos>,
    maybe_listener: Option<DdsListener>,
    phantom : PhantomData<T>,
}

impl <T>WriterBuilder<T> where T: TopicType {
    pub fn new() -> Self {
        Self {
            maybe_qos: None,
            maybe_listener: None,
            phantom: PhantomData,
        }
    }

    pub fn with_qos(mut self, qos : DdsQos) -> Self {
        self.maybe_qos = Some(qos);
        self
    }

    pub fn with_listener(mut self, listener : DdsListener) -> Self {
        self.maybe_listener = Some(listener);
        self
    }

    pub fn create(self,  
        entity: &dyn DdsWritable,
        topic: DdsTopic<T>) -> Result<DdsWriter<T>, DDSError> {
            DdsWriter::create(entity, topic, self.maybe_qos, self.maybe_listener)
        }
}

pub struct DdsWriter<T: Sized + TopicType>(
    DdsEntity,
    Option<DdsListener>,
    PhantomData<T>,
);

impl<'a, T> DdsWriter<T>
where
    T: Sized + TopicType,
{
    pub fn create(
        entity: &dyn DdsWritable,
        topic: DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
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
                    PhantomData,
                ))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn write_to_entity(entity: &DdsEntity, msg: std::sync::Arc<T>) -> Result<(), DDSError> {
        unsafe {
            let sample = Sample::<T>::from(msg);
            let sample = &sample as *const Sample<T>;
            let sample = sample as *const ::std::os::raw::c_void;
            let ret = dds_write(entity.entity(), sample);
            if ret >= 0 {
                Ok(())
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn write(&mut self, msg: std::sync::Arc<T>) -> Result<(), DDSError> {
        Self::write_to_entity(&self.0, msg)

    }

    // Loan memory buffers for zero copy operation
    pub fn loan(&mut self, buf: &mut SampleBuffer<T>) -> Result<(), DDSError> {

        let (voidp, _) = unsafe {buf.as_mut_ptr()};
        let voidpp = voidp as *mut *mut c_void;
        let res = unsafe {
            dds_loan_shared_memory_buffer(self.0.entity(), buf.len() as size_t, voidpp)
        };
        if res == 0 {
            Ok(())        
        } else {
            Err(DDSError::from(res))
        } 
    }

    pub fn set_listener(&mut self, listener: DdsListener) -> Result<(), DDSError> {
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

impl<'a, T> Entity for DdsWriter<T>
where
    T: std::marker::Sized + TopicType,
{
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

impl<'a, T> Drop for DdsWriter<T>
where
    T: std::marker::Sized + TopicType,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.0.entity()).into();
            if DDSError::DdsOk != ret && DDSError::AlreadyDeleted != ret {
                //panic!("cannot delete Writer: {}", ret);
            }
        }
    }
}
