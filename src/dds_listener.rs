use crate::error::DDSError;
use cyclonedds_sys::dds_listener_t;
use cyclonedds_sys::*;
use std::convert::From;

/// The callbacks are in a different structure that is always
/// heap allocated.
struct Callbacks {
    // Callbacks for readers
    on_sample_lost: Option<Box<dyn FnMut(dds_sample_lost_status_t)>>,
    on_data_available: Option<Box<dyn FnMut(dds_entity_t)>>,
    on_sample_rejected: Option<Box<dyn FnMut(dds_entity_t, dds_sample_rejected_status_t)>>,
    on_liveliness_changed: Option<Box<dyn FnMut(dds_entity_t, dds_liveliness_changed_status_t)>>,
    on_requested_deadline_missed:
        Option<Box<dyn FnMut(dds_entity_t, dds_requested_deadline_missed_status_t)>>,
    on_requested_incompatible_qos:
        Option<Box<dyn FnMut(dds_entity_t, dds_requested_incompatible_qos_status_t)>>,
    on_subscription_matched:
        Option<Box<dyn FnMut(dds_entity_t, dds_subscription_matched_status_t)>>,

    //callbacks for writers
    on_liveliness_lost: Option<Box<dyn FnMut(dds_entity_t, dds_liveliness_lost_status_t)>>,
    on_offered_deadline_missed:
        Option<Box<dyn FnMut(dds_entity_t, dds_offered_deadline_missed_status_t)>>,
    on_offered_incompatible_qos:
        Option<Box<dyn FnMut(dds_entity_t, dds_offered_incompatible_qos_status_t)>>,
    on_publication_matched: Option<Box<dyn FnMut(dds_entity_t, dds_publication_matched_status_t)>>,

    on_inconsistent_topic: Option<Box<dyn FnMut(dds_entity_t, dds_inconsistent_topic_status_t)>>,
    on_data_on_readers: Option<Box<dyn FnMut(dds_entity_t)>>,
}

impl Default for Callbacks {
    fn default() -> Self {
        Self {
            on_sample_lost: None,
            on_data_available: None,
            on_sample_rejected: None,
            on_liveliness_changed: None,
            on_requested_deadline_missed: None,
            on_requested_incompatible_qos: None,
            on_subscription_matched: None,
            on_liveliness_lost: None,
            on_offered_deadline_missed: None,
            on_offered_incompatible_qos: None,
            on_publication_matched: None,
            on_inconsistent_topic: None,
            on_data_on_readers: None,
        }
    }
}

pub struct DdsListener {
    listener: Option<*mut dds_listener_t>,
    callbacks: Option<Box<Callbacks>>,
    raw_ptr: Option<*mut Callbacks>,
}

impl DdsListener {
    pub fn new() -> Self {
        Self {
            listener: None,
            callbacks: Some(Box::new(Callbacks::default())),
            raw_ptr: None,
        }
    }
}

impl From<DdsListener> for *const dds_listener_t {
    fn from(listener: DdsListener) -> Self {
        if let Some(listener) = listener.listener {
            listener
        } else {
            panic!("Attempt to convert from unitialized listener");
        }
    }
}

impl From<&DdsListener> for *const dds_listener_t {
    fn from(listener: &DdsListener) -> Self {
        if let Some(listener) = listener.listener {
            listener
        } else {
            panic!("Attempt to convert from unitialized &listener");
        }
    }
}

/////
pub fn on_sample_lost<F, T>(reader: &mut DdsListener, callback: F)
where
    F: FnMut(dds_sample_lost_status_t),
    T: Sized + DDSGenType,
{
    unsafe {
        if let Some(listener) = reader.listener {
            dds_lset_sample_lost(listener, Some(call_sample_lost_closure::<F>));
        }
    }
}

unsafe extern "C" fn call_sample_lost_closure<F>(
    reader: dds_entity_t,
    status: dds_sample_lost_status_t,
    data: *mut std::ffi::c_void,
) where
    F: FnMut(dds_sample_lost_status_t),
{

}

impl DdsListener {
    // take ownership as we're going to do some bad stuff here.
    pub fn hook(mut self) -> Self {
        // we're going to grab the Boxed callbacks and keep them separately as
        // we will send a pointer to the callback array into C.

        // free the previous pointer if present
        if let Some(raw) = self.raw_ptr.take() {
            unsafe {
                // take ownership and free when out of scope
                Box::from_raw(raw);
            }
        }

        if let Some(b) = self.callbacks.take() {
            let raw = Box::into_raw(b);
            unsafe {
                let l = dds_create_listener(raw as *mut std::ffi::c_void);
                if !l.is_null() {
                    let callbacks_ptr = raw as *mut Callbacks;
                    let callbacks = &*callbacks_ptr;

                    self.register_callbacks(l, callbacks);

                    self.raw_ptr = Some(raw);
                    self.listener = Some(l);
                } else {
                    panic!("Error creating listener");
                }
            }
        } else {
            println!("No callbacks to take");
        }
        self
    }

    /// register the callbacks for the closures that have been set.DdsListener
    unsafe fn register_callbacks(&mut self, listener: *mut dds_listener_t, callbacks: &Callbacks) {
        if let Some(data_avail) = &callbacks.on_data_available {
            unsafe {
                println!("Listener hooked for data available");
                dds_lset_data_available(listener, Some(Self::call_data_available_closure));
            }
        } else {
            println!("No data_available_closure");
        }
    }
}

//////
impl DdsListener {
    pub fn on_data_available<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_data_available = Some(Box::new(callback));
        }

        self
    }

    unsafe extern "C" fn call_data_available_closure(
        reader: dds_entity_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback!");
        if let Some(avail) = &mut callbacks.on_data_available {
            avail(reader);
        }
    }
}
//////
pub fn on_sample_rejected<F, T>(reader: &mut DdsListener, callback: F)
where
    F: FnMut(dds_entity_t, dds_sample_rejected_status_t),
    T: Sized + DDSGenType,
{

}

pub fn on_liveliness_changed<F, T>(reader: &mut DdsListener, callback: F)
where
    F: FnMut(dds_entity_t, dds_liveliness_changed_status_t),
    T: Sized + DDSGenType,
{

}

pub fn on_requested_deadline_missed<F, T>(reader: &mut DdsListener, callback: F)
where
    F: FnMut(dds_entity_t, dds_requested_deadline_missed_status_t),
    T: Sized + DDSGenType,
{

}

pub fn on_requested_incompatible_qos<F, T>(reader: &mut DdsListener, callback: F)
where
    F: FnMut(dds_entity_t, dds_requested_incompatible_qos_status_t),
    T: Sized + DDSGenType,
{

}

pub fn on_subscription_matched<F, T>(reader: &mut DdsListener, callback: F)
where
    F: FnMut(dds_entity_t, dds_subscription_matched_status_t) + 'static,
    T: Sized + DDSGenType,
{

}

impl Drop for DdsListener {
    fn drop(&mut self) {
        println!("Dropping listener");

        if let Some(listener) = &self.listener {
            unsafe {
                dds_reset_listener(*listener);
                dds_delete_listener(*listener);
            }
        }
        // gain back control of the Callback structure
        if let Some(raw) = self.raw_ptr.take() {
            unsafe {
                // take ownership and free when out of scope
                Box::from_raw(raw);
            }
        }
    }
}
