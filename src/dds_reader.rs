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
use std::os::raw::c_void;
//use std::convert::TryInto;

pub use cyclonedds_sys::{DDSBox, DDSGenType, DdsDomainId, DdsEntity, DdsLoanedData};

use std::marker::PhantomData;

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsReadable, Entity};

pub struct DdsReader<'a, T: Sized + DDSGenType> {
    entity: DdsEntity,
    listener: Option<DdsListener<'a>>,
    _maybe_qos: Option<&'a DdsQos>,
    _phantom: PhantomData<*const T>,
    // The callback closures that can be attached to a reader
}

impl<'a, T> DdsReader<'a, T>
where
    T: Sized + DDSGenType,
{
    pub fn create(
        entity: &dyn DdsReadable,
        topic: &DdsTopic<T>,
        maybe_qos: Option<&'a DdsQos>,
        maybe_listener: Option<DdsListener<'a>>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_reader(
                entity.entity().entity(),
                topic.entity().entity(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsReader {
                    entity: DdsEntity::new(w),
                    listener: maybe_listener,
                    _maybe_qos: maybe_qos,
                    _phantom: PhantomData,
                })
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn set_listener(&mut self, listener: DdsListener<'a>) -> Result<(), DDSError> {
        unsafe {
            let refl = &listener;
            let rc = dds_set_listener(self.entity.entity(), refl.into());
            if rc == 0 {
                self.listener = Some(listener);
                Ok(())
            } else {
                Err(DDSError::from(rc))
            }
        }
    }

    /// Read a buffer given a dds_entity_t.  This is useful when you want to read data
    /// within a closure.
    pub fn read_from_entity(entity: DdsEntity) -> Result<DDSBox<T>, DDSError> {
        unsafe {
            let mut info = cyclonedds_sys::dds_sample_info::default();
            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_read(entity.entity(), voidpp, &mut info as *mut _, 1, 1);

            if ret >= 0 {
                if !voidp.is_null() && info.valid_data {
                    let buf = DDSBox::<T>::new_from_cyclone_allocated_struct(voidp as *mut T);
                    Ok(buf)
                } else {
                    Err(DDSError::OutOfResources)
                }
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn read(&self) -> Result<DdsLoanedData<T>, DDSError> {
        unsafe { cyclonedds_sys::read(self.entity()) }
    }

    pub fn take(&self) -> Result<DdsLoanedData<T>, DDSError> {
        unsafe { cyclonedds_sys::take(self.entity()) }
    }

    pub fn create_readcondition(
        &'a mut self,
        mask: StateMask,
    ) -> Result<DdsReadCondition<T>, DDSError> {
        DdsReadCondition::create(self, mask)
    }
}

impl<'a, T> Entity for DdsReader<'a, T>
where
    T: std::marker::Sized + DDSGenType,
{
    fn entity(&self) -> &DdsEntity {
        &self.entity
    }
}

impl<'a, T> Drop for DdsReader<'a, T>
where
    T: Sized + DDSGenType,
{
    fn drop(&mut self) {
        unsafe {
            println!("Drop reader:{:?}", self.entity().entity());
            let ret: DDSError = cyclonedds_sys::dds_delete(self.entity.entity()).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete Reader: {}", ret);
            } else {
                //println!("Reader dropped");
            }
        }
    }
}

pub struct DdsReadCondition<'a, T: Sized + DDSGenType>(DdsEntity, &'a DdsReader<'a, T>);

impl<'a, T> DdsReadCondition<'a, T>
where
    T: Sized + DDSGenType,
{
    fn create(reader: &'a DdsReader<'a, T>, mask: StateMask) -> Result<Self, DDSError> {
        unsafe {
            let mask: u32 = *mask;
            let p = cyclonedds_sys::dds_create_readcondition(reader.entity().entity(), mask);
            if p > 0 {
                Ok(DdsReadCondition(DdsEntity::new(p), reader))
            } else {
                Err(DDSError::from(p))
            }
        }
    }
}

impl<'a, T> Entity for DdsReadCondition<'a, T>
where
    T: std::marker::Sized + DDSGenType,
{
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}
