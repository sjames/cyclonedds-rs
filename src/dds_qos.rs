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
use std::clone::Clone;
use std::convert::From;

pub use cyclonedds_sys::{
    dds_destination_order_kind, dds_durability_kind, dds_duration_t, dds_history_kind,
    dds_ignorelocal_kind, dds_liveliness_kind, dds_ownership_kind,
    dds_presentation_access_scope_kind, dds_reliability_kind,
};

#[derive(Debug)]
pub struct DdsQos(*mut dds_qos_t);

impl DdsQos {
    pub fn create() -> Result<Self, DDSError> {
        unsafe {
            let p = cyclonedds_sys::dds_create_qos();
            if p != std::ptr::null_mut() {
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

    pub fn set_durability(&mut self, durability: dds_durability_kind) {
        unsafe {
            dds_qset_durability(self.0, durability);
        }
    }

    pub fn set_history(&mut self, history: dds_history_kind, depth: i32) {
        unsafe {
            dds_qset_history(self.0, history, depth);
        }
    }

    pub fn set_resource_limits(
        &mut self,
        max_samples: i32,
        max_instances: i32,
        max_samples_per_instance: i32,
    ) {
        unsafe {
            dds_qset_resource_limits(self.0, max_samples, max_instances, max_samples_per_instance);
        }
    }

    pub fn set_presentation(
        &mut self,
        access_scope: dds_presentation_access_scope_kind,
        coherent_access: bool,
        ordered_access: bool,
    ) {
        unsafe {
            dds_qset_presentation(self.0, access_scope, coherent_access, ordered_access);
        }
    }

    pub fn set_lifespan(&mut self, lifespan: dds_duration_t) {
        unsafe {
            dds_qset_lifespan(self.0, lifespan);
        }
    }

    pub fn set_deadline(&mut self, deadline: dds_duration_t) {
        unsafe {
            dds_qset_deadline(self.0, deadline);
        }
    }

    pub fn set_latency_budget(&mut self, duration: dds_duration_t) {
        unsafe {
            dds_qset_latency_budget(self.0, duration);
        }
    }

    pub fn set_ownership(&mut self, kind: dds_ownership_kind) {
        unsafe {
            dds_qset_ownership(self.0, kind);
        }
    }

    pub fn set_ownership_strength(&mut self, value: i32) {
        unsafe {
            dds_qset_ownership_strength(self.0, value);
        }
    }

    pub fn set_liveliness(&mut self, kind: dds_liveliness_kind, lease_duration: dds_duration_t) {
        unsafe {
            dds_qset_liveliness(self.0, kind, lease_duration);
        }
    }

    pub fn set_time_based_filter(&mut self, minimum_separation: dds_duration_t) {
        unsafe {
            dds_qset_time_based_filter(self.0, minimum_separation);
        }
    }

    pub fn set_reliability(
        &mut self,
        kind: dds_reliability_kind,
        max_blocking_time: dds_duration_t,
    ) {
        unsafe {
            dds_qset_reliability(self.0, kind, max_blocking_time);
        }
    }

    pub fn set_transport_priority(&mut self, value: i32) {
        unsafe {
            dds_qset_transport_priority(self.0, value);
        }
    }

    pub fn set_destination_order(&mut self, kind: dds_destination_order_kind) {
        unsafe {
            dds_qset_destination_order(self.0, kind);
        }
    }

    pub fn set_writer_data_lifecycle(&mut self, autodispose: bool) {
        unsafe {
            dds_qset_writer_data_lifecycle(self.0, autodispose);
        }
    }

    pub fn set_reader_data_lifecycle(
        &mut self,
        autopurge_nowriter_samples_delay: dds_duration_t,
        autopurge_disposed_samples_delay: dds_duration_t,
    ) {
        unsafe {
            dds_qset_reader_data_lifecycle(
                self.0,
                autopurge_nowriter_samples_delay,
                autopurge_disposed_samples_delay,
            );
        }
    }

    pub fn set_durability_service(
        &mut self,
        service_cleanup_delay: dds_duration_t,
        history_kind: dds_history_kind,
        history_depth: i32,
        max_samples: i32,
        max_instances: i32,
        max_samples_per_instance: i32,
    ) {
        unsafe {
            dds_qset_durability_service(
                self.0,
                service_cleanup_delay,
                history_kind,
                history_depth,
                max_samples,
                max_instances,
                max_samples_per_instance,
            );
        }
    }

    pub fn set_ignorelocal(&mut self, ignore: dds_ignorelocal_kind) {
        unsafe {
            dds_qset_ignorelocal(self.0, ignore);
        }
    }

    pub fn set_partition( &mut self, name: &std::ffi::CStr) {
        unsafe {
            dds_qset_partition1(self.0,
                name.as_ptr(),
                )
        }
    }
    //TODO:  Not implementing any getters for now

    
}

impl Drop for DdsQos {
    fn drop(&mut self) {
        unsafe { dds_delete_qos(self.0) }
    }
}

impl PartialEq for DdsQos {
    fn eq(&self, other: &Self) -> bool {
        unsafe {
            println!("Dropping");
            dds_qos_equal(self.0, other.0)
        }
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
                panic!("dds_copy_qos failed. Panicing as Clone should not fail");
            }
        }
    }
}

impl From<DdsQos> for *const dds_qos_t {
    fn from(qos: DdsQos) -> Self {
        qos.0
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
            let _c = qos.clone();
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
            qos.set_durability(dds_durability_kind::DDS_DURABILITY_VOLATILE);
            qos.set_history(dds_history_kind::DDS_HISTORY_KEEP_LAST, 3);
            qos.set_resource_limits(10, 1, 10);
            qos.set_presentation(
                dds_presentation_access_scope_kind::DDS_PRESENTATION_INSTANCE,
                false,
                false,
            );
            qos.set_lifespan(100);
            qos.set_deadline(100);
            qos.set_latency_budget(1000);
            qos.set_ownership(dds_ownership_kind::DDS_OWNERSHIP_EXCLUSIVE);
            qos.set_ownership_strength(1000);
            qos.set_liveliness(dds_liveliness_kind::DDS_LIVELINESS_AUTOMATIC, 10000);
            qos.set_time_based_filter(1000);
            qos.set_reliability(dds_reliability_kind::DDS_RELIABILITY_RELIABLE, 100);
            qos.set_transport_priority(1000);
            qos.set_destination_order(
                dds_destination_order_kind::DDS_DESTINATIONORDER_BY_RECEPTION_TIMESTAMP,
            );
            qos.set_writer_data_lifecycle(true);
            qos.set_reader_data_lifecycle(100, 100);
            qos.set_durability_service(0, dds_history_kind::DDS_HISTORY_KEEP_LAST, 3, 3, 3, 3);
        } else {
            assert!(false);
        }
    }
}
