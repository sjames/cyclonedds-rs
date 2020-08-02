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

pub use cyclonedds_sys::{DDSBox, DDSGenType, DdsDomainId, DdsEntity, DdsLoanedData};

use std::marker::PhantomData;

use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos,
    dds_subscriber::DdsSubscriber, dds_topic::DdsTopic, DdsReadable,
};

pub struct DdsReader<T: Sized + DDSGenType> {
    entity: dds_entity_t,
    listener: Option<DdsListener>,
    _phantom: PhantomData<*const T>,
    // The callback closures that can be attached to a reader
}

impl<T> DdsReader<T>
where
    T: Sized + DDSGenType,
{
    pub fn create(
        entity: &dyn DdsReadable,
        topic: &DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_reader(
                entity.entity(),
                topic.into(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsReader {
                    entity: w,
                    listener: maybe_listener,
                    _phantom: PhantomData,
                })
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn set_listener(&mut self, listener: DdsListener) -> Result<(), DDSError> {
        unsafe {
            let refl = &listener;
            let rc = dds_set_listener(self.entity, refl.into());
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

            let ret = dds_read(entity, voidpp, &mut info as *mut _, 1, 1);

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
        unsafe {
            let mut info: dds_sample_info = dds_sample_info::default();
            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_read(self.entity, voidpp, &mut info as *mut _, 1, 1);

            println!("Read returns pointer {:?}", voidpp);

            if ret >= 0 {
                if !voidp.is_null() && info.valid_data {
                    let ptr_to_ts: *const *const T = voidpp as *const *const T;
                    let data = DdsLoanedData::new(ptr_to_ts, self.entity, 1);
                    Ok(data)
                } else {
                    Err(DDSError::OutOfResources)
                }
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn take(&self) -> Result<DdsLoanedData<T>, DDSError> {
        unsafe {
            let mut info: dds_sample_info = dds_sample_info::default();
            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_take(self.entity, voidpp, &mut info as *mut _, 1, 1);

            //println!("Read returns pointer {:?}",voidpp);

            if ret >= 0 {
                if !voidp.is_null() && info.valid_data {
                    let ptr_to_ts: *const *const T = voidpp as *const *const T;
                    let data = DdsLoanedData::new(ptr_to_ts, self.entity, 1);
                    Ok(data)
                } else {
                    Err(DDSError::OutOfResources)
                }
            } else {
                Err(DDSError::from(ret))
            }
        }
    }

    pub fn entity(&self) -> DdsEntity {
        self.into()
    }
}

impl<T> From<DdsReader<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(reader: DdsReader<T>) -> Self {
        reader.entity
    }
}

impl<T> From<&DdsReader<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(reader: &DdsReader<T>) -> Self {
        reader.entity
    }
}

impl<T> Drop for DdsReader<T>
where
    T: Sized + DDSGenType,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.entity).into();
            if DDSError::DdsOk != ret {
                panic!("cannot delete Reader: {}", ret);
            } else {
                //println!("Reader dropped");
            }
        }
    }
}
