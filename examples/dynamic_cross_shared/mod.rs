// Shared by examples/dynamic_cross_writer.rs and examples/dynamic_cross_reader.rs, included
// via #[path] rather than left as a direct top-level examples/*.rs file so Cargo's example
// auto-discovery doesn't also try to build this (which has no main()) as its own example.
//
// Both binaries must build the *identical* type (same member names, kinds, and order) for
// the two processes to end up considering it the same wire type - this is the one place that
// definition lives, so there's only one to keep in sync.

// Each of the two consumers (dynamic_cross_writer.rs / dynamic_cross_reader.rs) only calls
// half of these functions - every compilation sees the other half as unused.
#![allow(dead_code)]

use cyclonedds_rs::*;

pub const TOPIC_NAME: &str = "dynamic_cross_process_test_topic";

pub fn build_type(participant: &DdsParticipant) -> Result<DdsDynamicType, DDSError> {
    DynamicTypeBuilder::new_struct("cyclonedds_rs::examples::DynamicCrossProcess")
        .member("id", DynamicKind::U32)
        .key("id")
        .member("value", DynamicKind::F64)
        .member("label", DynamicKind::String)
        .build(participant)
}

pub fn reliable_qos() -> DdsQos {
    let mut qos = DdsQos::create().expect("create qos");
    qos.set_reliability(
        dds_reliability_kind::DDS_RELIABILITY_RELIABLE,
        std::time::Duration::from_secs(10),
    );
    qos.set_history(dds_history_kind::DDS_HISTORY_KEEP_ALL, 0);
    qos
}

pub fn expected_label(id: u32) -> String {
    format!("cross-process-{}", id)
}

pub fn fill_sample(sample: &mut DynamicSample, id: u32) -> Result<(), DDSError> {
    sample.set("id", DynamicValue::U32(id))?;
    sample.set("value", DynamicValue::F64(id as f64 * 0.5))?;
    sample.set("label", DynamicValue::String(expected_label(id)))?;
    Ok(())
}

pub fn check_sample(sample: &DynamicSample, seen: &mut std::collections::HashSet<u32>) -> Result<(), String> {
    let id = match sample.get("id").map_err(|e| format!("get id: {:?}", e))? {
        DynamicValue::U32(v) => v,
        other => return Err(format!("wrong kind for id: {:?}", other)),
    };
    let value = match sample.get("value").map_err(|e| format!("get value: {:?}", e))? {
        DynamicValue::F64(v) => v,
        other => return Err(format!("wrong kind for value: {:?}", other)),
    };
    let label = match sample.get("label").map_err(|e| format!("get label: {:?}", e))? {
        DynamicValue::String(s) => s,
        other => return Err(format!("wrong kind for label: {:?}", other)),
    };

    if value != id as f64 * 0.5 {
        return Err(format!("corrupted value for id {}: {}", id, value));
    }
    if label != expected_label(id) {
        return Err(format!("corrupted label for id {}: {}", id, label));
    }
    if !seen.insert(id) {
        return Err(format!("duplicate id {}", id));
    }
    Ok(())
}
