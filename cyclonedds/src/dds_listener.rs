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

//! DdsListenerはDDSのイベントを受け取るための構造体です。
//! DdsListenerBuilderを使用して、必要なイベントのコールバックを登録し、build()メソッドでDdsListenerを生成します。
//!
//! # Example
//! ```no_run
//! use cyclonedds_rs::DdsListenerBuilder;
//! use cyclonedds_rs::error::ReaderError;
//! use futures_util::task::AtomicWaker;
//! use std::sync::{Arc, Mutex};
//!
//! let waker = Arc::new((AtomicWaker::new(), Mutex::new(None::<ReaderError>)));
//!
//! let listener = DdsListenerBuilder::new()
//!     .on_data_available({
//!         let waker = waker.clone();
//!         move |_entity| {
//!             waker.0.wake();
//!         }
//!     })
//!     .on_requested_deadline_missed({
//!         let waker = waker.clone();
//!         move |_entity, _status| {
//!             *waker.1.lock().unwrap() = Some(ReaderError::RequestedDeadLineMissed);
//!             waker.0.wake();
//!         }
//!     })
//!     .build();
//!
//! let _ = listener;
//! ```

use cyclonedds_sys::dds_listener_t;
use cyclonedds_sys::*;
use std::convert::From;

// コールバックは常にヒープ上に確保した別構造体で保持する。
#[derive(Default)]
struct Callbacks {
    // Reader 向けコールバック
    on_sample_lost: Option<Box<dyn FnMut(DdsEntity, dds_sample_lost_status_t) + 'static>>,
    on_data_available: Option<Box<dyn FnMut(DdsEntity) + 'static>>,
    on_sample_rejected: Option<Box<dyn FnMut(DdsEntity, dds_sample_rejected_status_t) + 'static>>,
    on_liveliness_changed:
        Option<Box<dyn FnMut(DdsEntity, dds_liveliness_changed_status_t) + 'static>>,
    on_requested_deadline_missed:
        Option<Box<dyn FnMut(DdsEntity, dds_requested_deadline_missed_status_t) + 'static>>,
    on_requested_incompatible_qos:
        Option<Box<dyn FnMut(DdsEntity, dds_requested_incompatible_qos_status_t) + 'static>>,
    on_subscription_matched:
        Option<Box<dyn FnMut(DdsEntity, dds_subscription_matched_status_t) + 'static>>,

    // Writer 向けコールバック
    on_liveliness_lost: Option<Box<dyn FnMut(DdsEntity, dds_liveliness_lost_status_t) + 'static>>,
    on_offered_deadline_missed:
        Option<Box<dyn FnMut(DdsEntity, dds_offered_deadline_missed_status_t) + 'static>>,
    on_offered_incompatible_qos:
        Option<Box<dyn FnMut(DdsEntity, dds_offered_incompatible_qos_status_t) + 'static>>,
    on_publication_matched:
        Option<Box<dyn FnMut(DdsEntity, dds_publication_matched_status_t) + 'static>>,

    on_inconsistent_topic:
        Option<Box<dyn FnMut(DdsEntity, dds_inconsistent_topic_status_t) + 'static>>,
    on_data_on_readers: Option<Box<dyn FnMut(DdsEntity) + 'static>>,
}

// [DdsListener]を別スレッドに渡すためにマーカートレイトを付与している
//
// # Safety
//
// 実用上Innerは作成時にHeap上に確保されて変更されないので、SendとSyncを実装しても安全
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

// DdsListenerの内部構造体
// dds_listenerインスタンスとそれに対応するコールバック構造体を保持する
struct Inner {
    listener: *mut dds_listener_t,
    raw_ptr: *mut Callbacks,
}

impl Inner {
    fn new(listener: *mut dds_listener_t, raw_ptr: *mut Callbacks) -> Self {
        Self { listener, raw_ptr }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            dds_reset_listener(self.listener);
            dds_delete_listener(self.listener);
        }
        unsafe {
            let _ = Box::from_raw(self.raw_ptr);
        }
    }
}

/// DDSのイベントを受け取るための構造体
#[derive(Clone)]
pub struct DdsListener {
    inner: std::sync::Arc<Inner>,
}

impl DdsListener {
    fn new(callbacks: Callbacks) -> Self {
        // heapに移動
        let callbacks = Box::new(callbacks);

        let inner = unsafe {
            let callbacks_ptr = Box::into_raw(callbacks);
            let l = dds_create_listener(callbacks_ptr as *mut std::ffi::c_void);
            if l.is_null() {
                panic!("Error creating listener");
            }
            Self::register_callbacks(l, &*callbacks_ptr);
            Inner::new(l, callbacks_ptr)
        };

        DdsListener {
            inner: std::sync::Arc::new(inner),
        }
    }
}

impl From<&DdsListener> for *const dds_listener_t {
    fn from(listener: &DdsListener) -> Self {
        listener.inner.listener
    }
}

impl DdsListener {
    /// 設定済みクロージャに対応するコールバックを C Listener に登録する。
    unsafe fn register_callbacks(listener: *mut dds_listener_t, callbacks: &Callbacks) {
        if callbacks.on_data_available.is_some() {
            dds_lset_data_available(listener, Some(Self::call_data_available_closure));
        }
        if callbacks.on_sample_lost.is_some() {
            dds_lset_sample_lost(listener, Some(Self::call_sample_lost_closure));
        }

        if callbacks.on_sample_rejected.is_some() {
            dds_lset_sample_rejected(listener, Some(Self::call_sample_rejected_closure));
        }

        if callbacks.on_liveliness_changed.is_some() {
            dds_lset_liveliness_changed(listener, Some(Self::call_liveliness_changed_closure));
        }

        if callbacks.on_requested_deadline_missed.is_some() {
            dds_lset_requested_deadline_missed(
                listener,
                Some(Self::call_requested_deadline_missed_closure),
            );
        }
        if callbacks.on_requested_incompatible_qos.is_some() {
            dds_lset_requested_incompatible_qos(
                listener,
                Some(Self::call_requested_incompatible_qos_closure),
            );
        }
        if callbacks.on_subscription_matched.is_some() {
            dds_lset_subscription_matched(listener, Some(Self::call_subscription_matched_closure));
        }
        if callbacks.on_liveliness_lost.is_some() {
            dds_lset_liveliness_lost(listener, Some(Self::call_liveliness_lost_closure));
        }
        if callbacks.on_offered_deadline_missed.is_some() {
            dds_lset_offered_deadline_missed(
                listener,
                Some(Self::call_offered_deadline_missed_closure),
            );
        }
        if callbacks.on_offered_incompatible_qos.is_some() {
            dds_lset_offered_incompatible_qos(
                listener,
                Some(Self::call_offered_incompatible_qos_closure),
            );
        }
        if callbacks.on_publication_matched.is_some() {
            dds_lset_publication_matched(listener, Some(Self::call_publication_matched_closure));
        }
        if callbacks.on_inconsistent_topic.is_some() {
            dds_lset_inconsistent_topic(listener, Some(Self::call_inconsistent_topic_closure));
        }
        if callbacks.on_data_on_readers.is_some() {
            dds_lset_data_on_readers(listener, Some(Self::call_data_on_readers_closure));
        }
    }
}

impl DdsListener {
    unsafe extern "C" fn call_data_available_closure(
        reader: dds_entity_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(avail) = &mut callbacks.on_data_available {
            avail(DdsEntity::new(reader));
        }
    }

    unsafe extern "C" fn call_sample_lost_closure(
        reader: dds_entity_t,
        status: dds_sample_lost_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(lost) = &mut callbacks.on_sample_lost {
            lost(DdsEntity::new(reader), status);
        }
    }

    unsafe extern "C" fn call_sample_rejected_closure(
        reader: dds_entity_t,
        status: dds_sample_rejected_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(rejected) = &mut callbacks.on_sample_rejected {
            rejected(DdsEntity::new(reader), status);
        }
    }

    unsafe extern "C" fn call_liveliness_changed_closure(
        entity: dds_entity_t,
        status: dds_liveliness_changed_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(changed) = &mut callbacks.on_liveliness_changed {
            changed(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_requested_deadline_missed_closure(
        entity: dds_entity_t,
        status: dds_requested_deadline_missed_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(missed) = &mut callbacks.on_requested_deadline_missed {
            missed(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_requested_incompatible_qos_closure(
        entity: dds_entity_t,
        status: dds_requested_incompatible_qos_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(incompatible_qos) = &mut callbacks.on_requested_incompatible_qos {
            incompatible_qos(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_subscription_matched_closure(
        entity: dds_entity_t,
        status: dds_subscription_matched_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(matched) = &mut callbacks.on_subscription_matched {
            matched(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_liveliness_lost_closure(
        entity: dds_entity_t,
        status: dds_liveliness_lost_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(lost) = &mut callbacks.on_liveliness_lost {
            lost(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_offered_deadline_missed_closure(
        entity: dds_entity_t,
        status: dds_offered_deadline_missed_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(missed) = &mut callbacks.on_offered_deadline_missed {
            missed(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_offered_incompatible_qos_closure(
        entity: dds_entity_t,
        status: dds_offered_incompatible_qos_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(incompatible) = &mut callbacks.on_offered_incompatible_qos {
            incompatible(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_publication_matched_closure(
        entity: dds_entity_t,
        status: dds_publication_matched_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(matched) = &mut callbacks.on_publication_matched {
            matched(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_inconsistent_topic_closure(
        entity: dds_entity_t,
        status: dds_inconsistent_topic_status_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(inconsistant) = &mut callbacks.on_inconsistent_topic {
            inconsistant(DdsEntity::new(entity), status);
        }
    }

    unsafe extern "C" fn call_data_on_readers_closure(
        entity: dds_entity_t,
        data: *mut std::ffi::c_void,
    ) {
        let callbacks_ptr = data as *mut Callbacks;
        let callbacks = &mut *callbacks_ptr;
        if let Some(data) = &mut callbacks.on_data_on_readers {
            data(DdsEntity::new(entity));
        }
    }
}

/// DdsListener を構築するためのビルダー
#[derive(Default)]
pub struct DdsListenerBuilder {
    callbacks: Callbacks,
}

impl DdsListenerBuilder {
    pub fn new() -> Self {
        Self {
            callbacks: Callbacks::default(),
        }
    }

    /// 設定済みのコールバックをフックして `DdsListener` を生成する。
    ///
    /// `self` を消費して `DdsListener` を返すため、
    /// 型システム上、同じビルダーで複数回 `build()` を呼ぶことはできない。
    #[must_use]
    pub fn build(self) -> DdsListener {
        DdsListener::new(self.callbacks)
    }

    /// Reader の `data_available` コールバックを登録する。
    ///
    /// データ到着時に `callback(entity)` が呼び出される。
    pub fn on_data_available<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity) + 'static,
    {
        self.callbacks.on_data_available = Some(Box::new(callback));
        self
    }

    /// Reader の `sample_lost` コールバックを登録する。
    pub fn on_sample_lost<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_sample_lost_status_t) + 'static,
    {
        self.callbacks.on_sample_lost = Some(Box::new(callback));
        self
    }

    /// Reader の `sample_rejected` コールバックを登録する。
    pub fn on_sample_rejected<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_sample_rejected_status_t) + 'static,
    {
        self.callbacks.on_sample_rejected = Some(Box::new(callback));
        self
    }

    /// Reader の `liveliness_changed` コールバックを登録する。
    ///
    /// participantの参加、離脱で呼び出される
    pub fn on_liveliness_changed<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_liveliness_changed_status_t) + 'static,
    {
        self.callbacks.on_liveliness_changed = Some(Box::new(callback));
        self
    }

    /// Reader の `requested_deadline_missed` コールバックを登録する。
    pub fn on_requested_deadline_missed<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_requested_deadline_missed_status_t) + 'static,
    {
        self.callbacks.on_requested_deadline_missed = Some(Box::new(callback));
        self
    }

    /// Reader の `requested_incompatible_qos` コールバックを登録する。
    pub fn on_requested_incompatible_qos<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_requested_incompatible_qos_status_t) + 'static,
    {
        self.callbacks.on_requested_incompatible_qos = Some(Box::new(callback));
        self
    }

    /// Reader の `subscription_matched` コールバックを登録する。
    pub fn on_subscription_matched<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_subscription_matched_status_t) + 'static,
    {
        self.callbacks.on_subscription_matched = Some(Box::new(callback));

        self
    }

    /// Writer の `liveliness_lost` コールバックを登録する。
    pub fn on_liveliness_lost<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_liveliness_lost_status_t) + 'static,
    {
        self.callbacks.on_liveliness_lost = Some(Box::new(callback));

        self
    }

    /// Writer の `offered_deadline_missed` コールバックを登録する。
    pub fn on_offered_deadline_missed<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_offered_deadline_missed_status_t) + 'static,
    {
        self.callbacks.on_offered_deadline_missed = Some(Box::new(callback));

        self
    }

    /// Writer の `offered_incompatible_qos` コールバックを登録する。
    pub fn on_offered_incompatible_qos<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_offered_incompatible_qos_status_t) + 'static,
    {
        self.callbacks.on_offered_incompatible_qos = Some(Box::new(callback));

        self
    }

    /// Writer の `publication_matched` コールバックを登録する。
    pub fn on_publication_matched<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_publication_matched_status_t) + 'static,
    {
        self.callbacks.on_publication_matched = Some(Box::new(callback));

        self
    }

    /// Topic の `inconsistent_topic` コールバックを登録する。
    pub fn on_inconsistent_topic<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity, dds_inconsistent_topic_status_t) + 'static,
    {
        self.callbacks.on_inconsistent_topic = Some(Box::new(callback));

        self
    }

    /// Subscriber の `data_on_readers` コールバックを登録する。
    pub fn on_data_on_readers<F>(mut self, callback: F) -> Self
    where
        F: FnMut(DdsEntity) + 'static,
    {
        self.callbacks.on_data_on_readers = Some(Box::new(callback));

        self
    }
}
