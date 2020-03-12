use crate::error::DDSError;
use cyclonedds_sys::*;
use std::convert::From;
use std::os::raw::c_void;

pub use cyclonedds_sys::{dds_domainid_t, dds_entity_t, dds_sample_info, DDSBox, DDSGenType};
pub use either::Either;
use std::marker::PhantomData;

use crate::{
    dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos,
    dds_subscriber::DdsSubscriber, dds_topic::DdsTopic,
};

pub struct DdsReader<T: Sized + DDSGenType>(
    dds_entity_t,
    Option<*mut dds_listener_t>,
    PhantomData<*const T>,
);

impl<T> DdsReader<T>
where
    T: Sized + DDSGenType,
{
    pub fn create(
        entity: Either<&DdsParticipant, &DdsSubscriber>,
        topic: &DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_reader(
                entity.either(|l| l.into(), |r| r.into()),
                topic.into(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsReader(w, None, PhantomData))
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    pub fn read(&self) -> Result<DDSBox<T>, DDSError> {
        unsafe {
            let mut info: dds_sample_info = dds_sample_info::default();
            // set to null pointer to ask cyclone to allocate the buffer. All received
            // data will need to be allocated by cyclone
            let mut voidp: *mut c_void = std::ptr::null::<T>() as *mut c_void;
            let voidpp: *mut *mut c_void = &mut voidp;

            let ret = dds_read(self.0, voidpp, &mut info as *mut _, 1, 1);

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
}

impl<T> From<DdsReader<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(reader: DdsReader<T>) -> Self {
        reader.0
    }
}

impl<T> From<&DdsReader<T>> for dds_entity_t
where
    T: Sized + DDSGenType,
{
    fn from(reader: &DdsReader<T>) -> Self {
        reader.0
    }
}

/*
typedef void (*dds_on_sample_lost_fn) (dds_entity_t reader, const dds_sample_lost_status_t status, void* arg);
typedef void (*dds_on_data_available_fn) (dds_entity_t reader, void* arg);
typedef void (*dds_on_sample_rejected_fn) (dds_entity_t reader, const dds_sample_rejected_status_t status, void* arg);
typedef void (*dds_on_liveliness_changed_fn) (dds_entity_t reader, const dds_liveliness_changed_status_t status, void* arg);
typedef void (*dds_on_requested_deadline_missed_fn) (dds_entity_t reader, const dds_requested_deadline_missed_status_t status, void* arg);
typedef void (*dds_on_requested_incompatible_qos_fn) (dds_entity_t reader, const dds_requested_incompatible_qos_status_t status, void* arg);
typedef void (*dds_on_subscription_matched_fn) (dds_entity_t reader, const dds_subscription_matched_status_t  status, void* arg);
*/

pub fn on_sample_lost<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_sample_lost_status_t) + 'static,
    T: Sized + DDSGenType,
{
    unsafe {
        if let Some(reader) = reader.1 {
            dds_lset_sample_lost(reader, Some(call_sample_lost_closure::<F>));
        }
    }
}

unsafe extern "C" fn call_sample_lost_closure<F>(
    reader: dds_entity_t,
    status : dds_sample_lost_status_t,
    data: *mut std::ffi::c_void,
) where
    F: FnMut(dds_sample_lost_status_t),
{

}

pub fn on_data_available<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_entity_t) + 'static,
    T: Sized + DDSGenType,
{

}

pub fn on_sample_rejected<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_entity_t, dds_sample_rejected_status_t) + 'static,
    T: Sized + DDSGenType,
{

}

pub fn on_liveliness_changed<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_entity_t, dds_liveliness_changed_status_t) + 'static,
    T: Sized + DDSGenType,
{

}

pub fn on_requested_deadline_missed<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_entity_t, dds_requested_deadline_missed_status_t) + 'static,
    T: Sized + DDSGenType,
{

}

pub fn on_requested_incompatible_qos<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_entity_t, dds_requested_incompatible_qos_status_t) + 'static,
    T: Sized + DDSGenType,
{

}

pub fn on_subscription_matched<F, T>(reader: &mut DdsReader<T>, callback: F)
where
    F: FnMut(dds_entity_t, dds_subscription_matched_status_t) + 'static,
    T: Sized + DDSGenType,
{

}
