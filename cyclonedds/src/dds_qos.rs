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

use cyclonedds_sys::{dds_qos_t, *};
use std::convert::From;
use std::time::Duration;
use std::{clone::Clone, fmt::Debug};

pub use cyclonedds_sys::{
    dds_destination_order_kind, dds_durability_kind, dds_duration_t, dds_history_kind,
    dds_ignorelocal_kind, dds_liveliness_kind, dds_ownership_kind,
    dds_presentation_access_scope_kind, dds_reliability_kind,
};

/// Safety Check:
/// The dds_qos_t pointer is not accessible externally. I'm assuming the QoS structure created
/// by Cyclone is Sendable here.
unsafe impl Send for DdsQos {}

pub struct DdsQos(*mut dds_qos_t);

impl DdsQos {
    fn from_dds_duration(value: dds_duration_t) -> Duration {
        if value <= 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(value as u64)
        }
    }

    pub fn create() -> Result<Self, DDSError> {
        unsafe {
            let p = cyclonedds_sys::dds_create_qos();
            if !p.is_null() {
                Ok(DdsQos(p))
            } else {
                Err(DDSError::OutOfResources)
            }
        }
    }

    pub fn merge(&mut self, src: &Self) {
        unsafe {
            dds_merge_qos(self.0, src.0);
        }
    }

    pub fn set_durability(&mut self, durability: dds_durability_kind) -> &mut Self {
        unsafe {
            dds_qset_durability(self.0, durability);
        }
        self
    }

    /// 信頼性のための再送用バッファのサイズを設定する
    ///
    /// 同期のための履歴保持は [Self::set_durability_service] を利用すること
    pub fn set_history(&mut self, history: dds_history_kind, depth: i32) -> &mut Self {
        unsafe {
            dds_qset_history(self.0, history, depth);
        }
        self
    }

    pub fn set_resource_limits(
        &mut self,
        max_samples: i32,
        max_instances: i32,
        max_samples_per_instance: i32,
    ) -> &mut Self {
        unsafe {
            dds_qset_resource_limits(self.0, max_samples, max_instances, max_samples_per_instance);
        }
        self
    }

    pub fn set_presentation(
        &mut self,
        access_scope: dds_presentation_access_scope_kind,
        coherent_access: bool,
        ordered_access: bool,
    ) -> &mut Self {
        unsafe {
            dds_qset_presentation(self.0, access_scope, coherent_access, ordered_access);
        }
        self
    }

    pub fn set_lifespan(&mut self, lifespan: std::time::Duration) -> &mut Self {
        unsafe {
            dds_qset_lifespan(self.0, lifespan.as_nanos() as i64);
        }
        self
    }

    pub fn set_deadline(&mut self, deadline: std::time::Duration) -> &mut Self {
        unsafe {
            dds_qset_deadline(self.0, deadline.as_nanos() as i64);
        }
        self
    }

    pub fn set_latency_budget(&mut self, duration: dds_duration_t) -> &mut Self {
        unsafe {
            dds_qset_latency_budget(self.0, duration);
        }
        self
    }

    pub fn set_ownership(&mut self, kind: dds_ownership_kind) -> &mut Self {
        unsafe {
            dds_qset_ownership(self.0, kind);
        }
        self
    }

    pub fn set_ownership_strength(&mut self, value: i32) -> &mut Self {
        unsafe {
            dds_qset_ownership_strength(self.0, value);
        }
        self
    }

    pub fn set_liveliness(
        &mut self,
        kind: dds_liveliness_kind,
        lease_duration: dds_duration_t,
    ) -> &mut Self {
        unsafe {
            dds_qset_liveliness(self.0, kind, lease_duration);
        }
        self
    }

    pub fn set_time_based_filter(&mut self, minimum_separation: dds_duration_t) -> &mut Self {
        unsafe {
            dds_qset_time_based_filter(self.0, minimum_separation);
        }
        self
    }

    pub fn set_reliability(
        &mut self,
        kind: dds_reliability_kind,
        max_blocking_time: std::time::Duration,
    ) -> &mut Self {
        unsafe {
            dds_qset_reliability(self.0, kind, max_blocking_time.as_nanos() as i64);
        }
        self
    }

    pub fn set_transport_priority(&mut self, value: i32) -> &mut Self {
        unsafe {
            dds_qset_transport_priority(self.0, value);
        }
        self
    }

    pub fn set_destination_order(&mut self, kind: dds_destination_order_kind) -> &mut Self {
        unsafe {
            dds_qset_destination_order(self.0, kind);
        }
        self
    }

    pub fn set_writer_data_lifecycle(&mut self, autodispose: bool) -> &mut Self {
        unsafe {
            dds_qset_writer_data_lifecycle(self.0, autodispose);
        }
        self
    }

    pub fn set_reader_data_lifecycle(
        &mut self,
        autopurge_nowriter_samples_delay: dds_duration_t,
        autopurge_disposed_samples_delay: dds_duration_t,
    ) -> &mut Self {
        unsafe {
            dds_qset_reader_data_lifecycle(
                self.0,
                autopurge_nowriter_samples_delay,
                autopurge_disposed_samples_delay,
            );
        }
        self
    }

    /// 同期（あとから参加しても過去配信データを受信できる）に関わるQoS設定。
    ///
    /// CycloneDDSでは、`TRANSIENT_LOCAL` の永続性レベルにおいて、あとから参加した
    /// Reader向けの履歴保持設定をこのQoSで行う。一般的な解釈では [Self::set_history] に
    /// 期待される機能を、こちらで担っている。
    /// OMGのDCPS QoS定義では、`TRANSIENT` または `PERSISTENT` の永続性レベルにおいて、
    /// データを管理する「仮想的なサービス」の設定として定義されている。
    ///
    /// この仕様差は、CycloneDDSメンテナが `DURABILITY_SERVICE` QoS を
    /// 「接続の確立時（または再確立時）のデータ同期」の設定、`HISTORY` QoS を
    /// 「接続確立後の再送用バッファサイズ」の設定として、意図的に使い分けているためであり、
    /// ライブデータの全数配信のためのKEEP_ALLで保証しつつ、
    /// 後から参加者には直近n件のみといった実用上有用な設定をサポートできる設計となっている。
    ///
    /// Reference: https://github.com/eclipse-cyclonedds/cyclonedds/issues/49
    pub fn set_durability_service(
        &mut self,
        service_cleanup_delay: Duration,
        history_kind: dds_history_kind,
        history_depth: i32,
        max_samples: i32,
        max_instances: i32,
        max_samples_per_instance: i32,
    ) -> &mut Self {
        unsafe {
            dds_qset_durability_service(
                self.0,
                service_cleanup_delay.as_nanos() as i64,
                history_kind,
                history_depth,
                max_samples,
                max_instances,
                max_samples_per_instance,
            );
        }
        self
    }

    pub fn set_ignorelocal(&mut self, ignore: dds_ignorelocal_kind) -> &mut Self {
        unsafe {
            dds_qset_ignorelocal(self.0, ignore);
        }
        self
    }

    pub fn set_partition(&mut self, name: &std::ffi::CStr) -> &mut Self {
        unsafe { dds_qset_partition1(self.0, name.as_ptr()) }
        self
    }

    pub fn durability(&self) -> dds_durability_kind {
        let mut kind = dds_durability_kind::DDS_DURABILITY_VOLATILE;
        unsafe {
            let _ = dds_qget_durability(self.0, &mut kind as *mut _);
        }
        kind
    }

    pub fn history(&self) -> (dds_history_kind, i32) {
        let mut depth = 1;
        let mut kind = dds_history_kind::DDS_HISTORY_KEEP_LAST;
        unsafe {
            let _ = dds_qget_history(self.0, &mut kind as *mut _, &mut depth as *mut i32);
        }
        (kind, depth)
    }

    pub fn reliability(&self) -> (dds_reliability_kind, std::time::Duration) {
        let mut max_blocking_time = 0;
        let mut kind = dds_reliability_kind::DDS_RELIABILITY_BEST_EFFORT;
        unsafe {
            let _ = dds_qget_reliability(
                self.0,
                &mut kind as *mut _,
                &mut max_blocking_time as *mut _,
            );
        }
        (kind, Self::from_dds_duration(max_blocking_time))
    }

    pub fn lifespan(&self) -> std::time::Duration {
        let mut lifespan = 0;
        unsafe {
            let _ = dds_qget_lifespan(self.0, &mut lifespan as *mut _);
        }
        Self::from_dds_duration(lifespan)
    }

    pub fn deadline(&self) -> std::time::Duration {
        let mut deadline = 0;
        unsafe {
            let _ = dds_qget_deadline(self.0, &mut deadline as *mut _);
        }
        Self::from_dds_duration(deadline)
    }

    pub fn liveliness(&self) -> (dds_liveliness_kind, std::time::Duration) {
        let mut lease_duration = 0;
        let mut kind = dds_liveliness_kind::DDS_LIVELINESS_AUTOMATIC;
        unsafe {
            let _ = dds_qget_liveliness(self.0, &mut kind as *mut _, &mut lease_duration as *mut _);
        }
        (kind, Self::from_dds_duration(lease_duration))
    }

    // 内部でポインタからDdsQosを作成する
    pub(crate) fn from_ptr(ptr: *mut dds_qos_t) -> Self {
        DdsQos(ptr)
    }

    // 借用したQosのポインタは開放しない
    fn forget(mut self) {
        self.0 = std::ptr::null_mut();
    }
}

impl Default for DdsQos {
    fn default() -> Self {
        DdsQos::create().expect("Unable to create DdsQos")
    }
}

impl Drop for DdsQos {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { dds_delete_qos(self.0) }
        }
    }
}

impl PartialEq for DdsQos {
    fn eq(&self, other: &Self) -> bool {
        unsafe { dds_qos_equal(self.0, other.0) }
    }
}

impl Eq for DdsQos {}

impl Clone for DdsQos {
    fn clone(&self) -> Self {
        unsafe {
            let q = dds_create_qos();
            let err: DDSError = dds_copy_qos(q, self.0).into();
            if let DDSError::DdsOk = err {
                DdsQos(q)
            } else {
                panic!("dds_copy_qos failed. Panicking as Clone should not fail");
            }
        }
    }
}

impl From<DdsQos> for *const dds_qos_t {
    fn from(mut qos: DdsQos) -> Self {
        let q = qos.0;
        // we need to forget the pointer here
        qos.0 = std::ptr::null_mut();
        // setting to zero will ensure drop will not deallocate it
        q
    }
}

impl Debug for DdsQos {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("DdsQos")
            .field("durability", &self.durability())
            .field("history", &self.history())
            .field("reliability", &self.reliability())
            .field("lifespan", &self.lifespan())
            .field("deadline", &self.deadline())
            .field("liveliness", &self.liveliness())
            .finish()
    }
}
/*
impl From<&mut DdsQos> for *const dds_qos_t {
    fn from(qos: &mut DdsQos) -> Self {
        let q = qos.0;
        // we need to forget the pointer here
        qos.0 = std::ptr::null_mut();
        // setting to zero will ensure drop will not deallocate it
        q
    }
}
*/

/// メッセージ履歴設定
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum History {
    /// 指定された数だけ過去のサンプルを保持する
    /// 想定インスタンス数はPolicy::SUPPORT_INSTANCESに依存する
    KeepLast(i32),
    /// すべてのサンプルを保持する
    KeepAll,
}

impl Default for History {
    fn default() -> Self {
        History::KeepLast(1)
    }
}

/// 到達保証設定
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Reliability {
    /// 信頼性あり。最大ブロッキング時間を指定する
    Reliable(Duration),
    /// ベストエフォート
    #[default]
    BestEffort,
}

/// 永続性設定
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Durability {
    /// Readerが起動している間のみデータを保持する
    #[default]
    Volatile,
    /// データをローカルに保存し、後から起動したReaderにも配信する
    TransientLocal,
}

/// 通信可否に関わる重要なQoS要素のみをまとめた構造体
///
/// QoSは実際には効果のない設定があり設定が煩雑で、
/// インスタンスに紐づく情報が含まれるため比較が難しいため
/// 実用上の比較や設定はこちらを利用することを推奨する
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Policy {
    pub history: History,
    pub reliability: Reliability,
    pub durability: Durability,
}

impl Policy {
    const SUPPORT_INSTANCES: i32 = 4;
    pub fn create_transient_local(history: i32, deadline: Option<Duration>) -> Self {
        Policy {
            history: History::KeepLast(history),
            reliability: Reliability::Reliable(deadline.unwrap_or(Duration::from_millis(100))),
            durability: Durability::TransientLocal,
        }
    }

    pub fn to_qos(&self) -> DdsQos {
        let mut qos = DdsQos::create().expect("Unable to create DdsQos");
        // History
        match self.history {
            History::KeepLast(depth) => {
                qos.set_history(dds_history_kind::DDS_HISTORY_KEEP_LAST, depth);
                let max_sample = depth * Self::SUPPORT_INSTANCES;
                qos.set_resource_limits(max_sample, Self::SUPPORT_INSTANCES, depth);
            }
            History::KeepAll => {
                qos.set_history(dds_history_kind::DDS_HISTORY_KEEP_ALL, 0);
            }
        }
        // Reliability
        match self.reliability {
            Reliability::Reliable(max_blocking_time) => {
                qos.set_reliability(
                    dds_reliability_kind::DDS_RELIABILITY_RELIABLE,
                    max_blocking_time,
                );
            }
            Reliability::BestEffort => {
                qos.set_reliability(
                    dds_reliability_kind::DDS_RELIABILITY_BEST_EFFORT,
                    Duration::from_nanos(0),
                );
            }
        }
        // Durability
        match self.durability {
            Durability::Volatile => {
                qos.set_durability(dds_durability_kind::DDS_DURABILITY_VOLATILE);
            }
            Durability::TransientLocal => {
                qos.set_durability(dds_durability_kind::DDS_DURABILITY_TRANSIENT_LOCAL);
                if self.reliability == Reliability::BestEffort {
                    // TransientLocal で BestEffort は非推奨なので Reliable に変更する
                    qos.set_reliability(
                        dds_reliability_kind::DDS_RELIABILITY_RELIABLE,
                        Duration::from_millis(100),
                    );
                }
            }
        }
        qos
    }
}

impl From<&DdsQos> for Policy {
    fn from(qos: &DdsQos) -> Self {
        let history = match qos.history().0 {
            dds_history_kind::DDS_HISTORY_KEEP_LAST => History::KeepLast(qos.history().1),
            dds_history_kind::DDS_HISTORY_KEEP_ALL => History::KeepAll,
        };
        let reliability = match qos.reliability().0 {
            dds_reliability_kind::DDS_RELIABILITY_RELIABLE => {
                Reliability::Reliable(qos.reliability().1)
            }
            dds_reliability_kind::DDS_RELIABILITY_BEST_EFFORT => Reliability::BestEffort,
        };
        let durability = match qos.durability() {
            dds_durability_kind::DDS_DURABILITY_VOLATILE => Durability::Volatile,
            _ => Durability::TransientLocal,
        };
        Policy {
            history,
            reliability,
            durability,
        }
    }
}

impl From<*mut dds_qos_t> for Policy {
    fn from(qos: *mut dds_qos_t) -> Self {
        if qos.is_null() {
            return Policy::default();
        }
        let q = DdsQos::from_ptr(qos);
        let p = Policy::from(&q);
        q.forget();
        p
    }
}

impl From<*const dds_qos_t> for Policy {
    fn from(qos: *const dds_qos_t) -> Self {
        Self::from(qos as *mut dds_qos_t)
    }
}

#[cfg(test)]
mod dds_qos_tests {
    use super::*;

    #[test]
    fn test_create_qos() {
        if let Ok(_qos) = DdsQos::create() {
        } else {
            assert!(false);
        }
    }
    #[test]
    fn test_clone_qos() {
        if let Ok(qos) = DdsQos::create() {
            let _c = qos;
        } else {
            assert!(false);
        }
    }

    #[test]
    fn test_merge_qos() {
        if let Ok(mut qos) = DdsQos::create() {
            let c = qos.clone();
            qos.merge(&c);
        } else {
            assert!(false);
        }
    }

    #[test]
    fn test_set() {
        if let Ok(mut qos) = DdsQos::create() {
            let _qos = qos
                .set_durability(dds_durability_kind::DDS_DURABILITY_VOLATILE)
                .set_history(dds_history_kind::DDS_HISTORY_KEEP_LAST, 3)
                .set_resource_limits(10, 1, 10)
                .set_presentation(
                    dds_presentation_access_scope_kind::DDS_PRESENTATION_INSTANCE,
                    false,
                    false,
                )
                .set_lifespan(std::time::Duration::from_nanos(100))
                .set_deadline(std::time::Duration::from_nanos(100))
                .set_latency_budget(1000)
                .set_ownership(dds_ownership_kind::DDS_OWNERSHIP_EXCLUSIVE)
                .set_ownership_strength(1000)
                .set_liveliness(dds_liveliness_kind::DDS_LIVELINESS_AUTOMATIC, 10000)
                .set_time_based_filter(1000)
                .set_reliability(
                    dds_reliability_kind::DDS_RELIABILITY_RELIABLE,
                    std::time::Duration::from_nanos(100),
                )
                .set_transport_priority(1000)
                .set_destination_order(
                    dds_destination_order_kind::DDS_DESTINATIONORDER_BY_RECEPTION_TIMESTAMP,
                )
                .set_writer_data_lifecycle(true)
                .set_reader_data_lifecycle(100, 100)
                .set_durability_service(
                    Duration::ZERO,
                    dds_history_kind::DDS_HISTORY_KEEP_LAST,
                    3,
                    3,
                    3,
                    3,
                )
                .set_partition(&std::ffi::CString::new("partition1").unwrap());
        } else {
            assert!(false);
        }
    }
}
