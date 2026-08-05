// End-to-end test for the Dynamic Type MVP (see docs/design/dynamic-types.md): define a
// struct type at runtime, publish samples built via DynamicSample, and read them back via
// DdsDynamicReader - exercising the whole pipeline (type descriptor -> register -> topic
// descriptor -> m_ops offset parsing -> raw write/read -> value round-trip) against a real
// participant, per the design doc's recommendation to validate this against live CycloneDDS
// rather than only unit-testing the opcode parser in isolation.
//
// Unlike tests/stress_test.rs, this doesn't touch SHM/PSMX at all - dynamic-type topics use
// CycloneDDS's own built-in default sertype via a plain dds_create_topic, the same path any
// ordinary (non-loan) write/read goes through.

use cyclonedds_rs::*;
use std::time::{Duration, Instant};

fn poll_take(reader: &DdsDynamicReader, want: usize, timeout: Duration) -> Vec<DynamicSample> {
    let deadline = Instant::now() + timeout;
    let mut got = Vec::new();
    while got.len() < want && Instant::now() < deadline {
        match reader.take(32) {
            Ok(mut samples) => got.append(&mut samples),
            Err(DDSError::NoData) => {}
            Err(e) => panic!("take() failed: {:?}", e),
        }
        if got.len() < want {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    got
}

#[test]
fn dynamic_type_round_trip() {
    let participant = DdsParticipant::create(Some(60), None, None).expect("create participant");

    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::DynamicSensor")
        .member("id", DynamicKind::U32)
        .key("id")
        .member("value", DynamicKind::F64)
        .member("label", DynamicKind::String)
        .build(&participant)
        .expect("build dynamic type");

    assert_eq!(dtype.type_name(), "cyclonedds_rs::test::DynamicSensor");
    assert!(dtype.is_key("id"));
    assert!(!dtype.is_key("value"));
    assert!(!dtype.is_key("label"));
    assert!(!dtype.is_key("no_such_field"));

    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let topic = DdsDynamicTopic::create(&participant, &dtype, "dynamic_sensor", None, None)
        .expect("create topic");

    let mut writer =
        DdsDynamicWriter::create(&publisher, &topic, None, None).expect("create writer");
    let reader = DdsDynamicReader::create(&subscriber, &topic, None, None).expect("create reader");

    const COUNT: u32 = 20;
    for i in 0..COUNT {
        let mut sample = writer.new_sample();
        sample.set("id", DynamicValue::U32(i)).expect("set id");
        sample
            .set("value", DynamicValue::F64(i as f64 * 1.5))
            .expect("set value");
        sample
            .set("label", DynamicValue::String(format!("sensor-{}", i)))
            .expect("set label");
        writer.write(&sample).expect("write");
    }

    let samples = poll_take(&reader, COUNT as usize, Duration::from_secs(10));
    assert_eq!(
        samples.len(),
        COUNT as usize,
        "expected {} samples, got {}",
        COUNT,
        samples.len()
    );

    let mut seen_ids = std::collections::HashSet::new();
    for sample in &samples {
        let id = match sample.get("id").expect("get id") {
            DynamicValue::U32(v) => v,
            other => panic!("wrong kind for id: {:?}", other),
        };
        let value = match sample.get("value").expect("get value") {
            DynamicValue::F64(v) => v,
            other => panic!("wrong kind for value: {:?}", other),
        };
        let label = match sample.get("label").expect("get label") {
            DynamicValue::String(s) => s,
            other => panic!("wrong kind for label: {:?}", other),
        };

        assert_eq!(value, id as f64 * 1.5, "corrupted value for id {}", id);
        assert_eq!(label, format!("sensor-{}", id), "corrupted label for id {}", id);
        assert!(seen_ids.insert(id), "duplicate id {}", id);
    }
    assert_eq!(seen_ids.len(), COUNT as usize);
}

#[test]
fn dynamic_sample_set_rejects_kind_mismatch() {
    let participant = DdsParticipant::create(Some(61), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::KindMismatch")
        .member("id", DynamicKind::U32)
        .build(&participant)
        .expect("build dynamic type");

    let mut sample = DynamicSample::new(&dtype);
    assert!(sample.set("id", DynamicValue::String("nope".into())).is_err());
    assert!(sample.set("no_such_field", DynamicValue::U32(1)).is_err());
    assert!(sample.set("id", DynamicValue::U32(42)).is_ok());
    assert_eq!(sample.get("id").unwrap(), DynamicValue::U32(42));
}
