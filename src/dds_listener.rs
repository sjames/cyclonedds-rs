use cyclonedds_sys::dds_listener_t;
use cyclonedds_sys::*;
use std::convert::From;

/*
 Each listener has its own set of callbacks.
*/

/// The callbacks are in a different structure that is always
/// heap allocated.
struct Callbacks {
    // Callbacks for readers
    on_sample_lost: Option<Box<dyn FnMut(dds_entity_t, dds_sample_lost_status_t)>>,
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

impl DdsListener {
    // take ownership as we're going to do some bad stuff here.
    pub fn hook(mut self) -> Self {
        // we're going to grab the Boxed callbacks and keep them separately as
        // we will send a pointer to the callback array into C. We convert the
        // pointer back to a box in the Drop function.

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
        if let Some(_) = &callbacks.on_data_available {
            println!("Listener hooked for data available");
            dds_lset_data_available(listener, Some(Self::call_data_available_closure));
        }
        if let Some(_) = &callbacks.on_sample_lost {
            dds_lset_sample_lost(listener, Some(Self::call_sample_lost_closure));
        }

        if let Some(_) = &callbacks.on_sample_rejected {
            dds_lset_sample_rejected(listener, Some(Self::call_sample_rejected_closure));
        }

        if let Some(_) = &callbacks.on_liveliness_changed {
            dds_lset_liveliness_changed(listener, Some(Self::call_liveliness_changed_closure));
        }

        if let Some(_) = &callbacks.on_requested_deadline_missed {
            dds_lset_requested_deadline_missed(
                listener,
                Some(Self::call_requested_deadline_missed_closure),
            );
        }

        if let Some(_) = &callbacks.on_requested_incompatible_qos {
            dds_lset_requested_incompatible_qos(
                listener,
                Some(Self::call_requested_incompatible_qos_closure),
            );
        }

        if let Some(_) = &callbacks.on_subscription_matched {
            dds_lset_subscription_matched(listener, Some(Self::call_subscription_matched_closure));
        }
        if let Some(_) = &callbacks.on_liveliness_lost {
            dds_lset_liveliness_lost(listener, Some(Self::call_liveliness_lost_closure));
        }
        if let Some(_) = &callbacks.on_offered_deadline_missed {
            dds_lset_offered_deadline_missed(
                listener,
                Some(Self::call_offered_deadline_missed_closure),
            );
        }
        if let Some(_) = &callbacks.on_offered_incompatible_qos {
            dds_lset_offered_incompatible_qos(
                listener,
                Some(Self::call_offered_incompatible_qos_closure),
            );
        }
        if let Some(_) = &callbacks.on_publication_matched {
            dds_lset_publication_matched(listener, Some(Self::call_publication_matched_closure));
        }
        if let Some(_) = &callbacks.on_inconsistent_topic {
            dds_lset_inconsistent_topic(listener, Some(Self::call_inconsistent_topic_closure));
        }
        if let Some(_) = &callbacks.on_data_on_readers {
            dds_lset_data_on_readers(listener, Some(Self::call_data_on_readers_closure));
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
        //        println!("C Callback!");
        if let Some(avail) = &mut callbacks.on_data_available {
            avail(reader);
        }
    }
}

impl DdsListener {
    /////
    pub fn on_sample_lost<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_sample_lost_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_sample_lost = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_sample_lost_closure(
        reader: dds_entity_t,
        status: dds_sample_lost_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - sample lost");
        if let Some(lost) = &mut callbacks.on_sample_lost {
            lost(reader, status);
        }
    }
}

impl DdsListener {
    //////
    pub fn on_sample_rejected<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_sample_rejected_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_sample_rejected = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_sample_rejected_closure(
        reader: dds_entity_t,
        status: dds_sample_rejected_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - sample rejected");
        if let Some(rejected) = &mut callbacks.on_sample_rejected {
            rejected(reader, status);
        }
    }
}

// Liveliness changed
impl DdsListener {
    pub fn on_liveliness_changed<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_liveliness_changed_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_liveliness_changed = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_liveliness_changed_closure(
        entity: dds_entity_t,
        status: dds_liveliness_changed_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - Liveliness changed");
        if let Some(changed) = &mut callbacks.on_liveliness_changed {
            changed(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_requested_deadline_missed<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_requested_deadline_missed_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_requested_deadline_missed = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_requested_deadline_missed_closure(
        entity: dds_entity_t,
        status: dds_requested_deadline_missed_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - requested deadline missed");
        if let Some(missed) = &mut callbacks.on_requested_deadline_missed {
            missed(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_requested_incompatible_qos<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_requested_incompatible_qos_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_requested_incompatible_qos = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_requested_incompatible_qos_closure(
        entity: dds_entity_t,
        status: dds_requested_incompatible_qos_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - requested incompatible QOS");
        if let Some(incompatible_qos) = &mut callbacks.on_requested_incompatible_qos {
            incompatible_qos(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_subscription_matched<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_subscription_matched_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_subscription_matched = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_subscription_matched_closure(
        entity: dds_entity_t,
        status: dds_subscription_matched_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - subscription matched");
        if let Some(matched) = &mut callbacks.on_subscription_matched {
            matched(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_liveliness_lost<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_liveliness_lost_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_liveliness_lost = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_liveliness_lost_closure(
        entity: dds_entity_t,
        status: dds_liveliness_lost_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - liveliness lost");
        if let Some(lost) = &mut callbacks.on_liveliness_lost {
            lost(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_offered_deadline_missed<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_offered_deadline_missed_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_offered_deadline_missed = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_offered_deadline_missed_closure(
        entity: dds_entity_t,
        status: dds_offered_deadline_missed_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - offered deadline missed");
        if let Some(missed) = &mut callbacks.on_offered_deadline_missed {
            missed(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_offered_incompatible_qos<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_offered_incompatible_qos_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_offered_incompatible_qos = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_offered_incompatible_qos_closure(
        entity: dds_entity_t,
        status: dds_offered_incompatible_qos_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - offered incompatible QOS");
        if let Some(incompatible) = &mut callbacks.on_offered_incompatible_qos {
            incompatible(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_publication_matched<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_publication_matched_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_publication_matched = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_publication_matched_closure(
        entity: dds_entity_t,
        status: dds_publication_matched_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - publication matched");
        if let Some(matched) = &mut callbacks.on_publication_matched {
            matched(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_inconsistent_topic<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t, dds_inconsistent_topic_status_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_inconsistent_topic = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_inconsistent_topic_closure(
        entity: dds_entity_t,
        status: dds_inconsistent_topic_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - inconsistent topic");
        if let Some(inconsistant) = &mut callbacks.on_inconsistent_topic {
            inconsistant(entity, status);
        }
    }
}

impl DdsListener {
    pub fn on_data_on_readers<F>(mut self, callback: F) -> Self
    where
        F: FnMut(dds_entity_t) + 'static,
    {
        if let Some(callbacks) = &mut self.callbacks {
            callbacks.on_data_on_readers = Some(Box::new(callback));
        }
        self
    }

    unsafe extern "C" fn call_data_on_readers_closure(
        entity: dds_entity_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        println!("C Callback - data on readers");
        if let Some(data) = &mut callbacks.on_data_on_readers {
            data(entity);
        }
    }
}

impl Drop for DdsListener {
    fn drop(&mut self) {
        println!("Dropping listener");

        // delete the listener so we are sure of not
        // getting any callbacks
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
