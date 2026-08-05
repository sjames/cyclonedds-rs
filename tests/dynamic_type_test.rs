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
use std::convert::TryInto;
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

/// Independently verifies the opcode-parsed field offsets (the part `layout::parse` exists
/// specifically to get right - see its module doc) against `std::mem::offset_of!` on a plain
/// `#[repr(C)]` Rust struct with equivalent fields. Rust's repr(C) layout algorithm is the
/// standard C ABI one, computed by the compiler with no knowledge of this crate's opcode
/// parser at all - so if CycloneDDS's own m_ops-derived offsets (recovered by that parser)
/// agree with it, that's real evidence the parser is extracting the right numbers, not just
/// numbers that are self-consistent with the rest of this crate's code.
#[test]
fn dynamic_layout_matches_repr_c_equivalent() {
    #[repr(C)]
    struct StaticEquivalent {
        id: u32,
        value: f64,
        label: *mut std::os::raw::c_char,
    }

    let participant = DdsParticipant::create(Some(62), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::ReprCCrossCheck")
        .member("id", DynamicKind::U32)
        .key("id")
        .member("value", DynamicKind::F64)
        .member("label", DynamicKind::String)
        .build(&participant)
        .expect("build dynamic type");

    assert_eq!(
        dtype.size() as usize,
        std::mem::size_of::<StaticEquivalent>(),
        "CycloneDDS's computed sample size doesn't match repr(C)'s"
    );
    assert_eq!(
        dtype.align() as usize,
        std::mem::align_of::<StaticEquivalent>(),
        "CycloneDDS's computed sample alignment doesn't match repr(C)'s"
    );

    let mut sample = DynamicSample::new(&dtype);
    sample.set("id", DynamicValue::U32(7)).expect("set id");
    sample
        .set("value", DynamicValue::F64(2.5))
        .expect("set value");
    sample
        .set("label", DynamicValue::String("x".into()))
        .expect("set label");

    let bytes = sample.as_bytes();
    let id_off = std::mem::offset_of!(StaticEquivalent, id);
    let value_off = std::mem::offset_of!(StaticEquivalent, value);
    let label_off = std::mem::offset_of!(StaticEquivalent, label);

    let id = u32::from_ne_bytes(bytes[id_off..id_off + 4].try_into().unwrap());
    assert_eq!(id, 7, "id not found at repr(C)'s offset ({})", id_off);

    let value = f64::from_ne_bytes(bytes[value_off..value_off + 8].try_into().unwrap());
    assert_eq!(value, 2.5, "value not found at repr(C)'s offset ({})", value_off);

    let ptr_size = std::mem::size_of::<*mut std::os::raw::c_char>();
    let ptr_bytes: [u8; 8] = {
        let mut b = [0u8; 8];
        b[..ptr_size].copy_from_slice(&bytes[label_off..label_off + ptr_size]);
        b
    };
    let label_ptr = usize::from_ne_bytes(ptr_bytes) as *const std::os::raw::c_char;
    assert!(
        !label_ptr.is_null(),
        "label pointer not found at repr(C)'s offset ({})",
        label_off
    );
    let label = unsafe { std::ffi::CStr::from_ptr(label_ptr) }
        .to_str()
        .unwrap();
    assert_eq!(label, "x", "wrong string content at repr(C)'s offset");
}

/// Empty string, a very long string, and every numeric kind at its extreme values -
/// in-memory only (DynamicSample::set()/get()), no DDS transport involved. Exercises the
/// string-allocation/free path (dds_string_alloc's size-in-characters convention -
/// alloc_dds_string in dds_dynamic_type.rs) at both ends of the length range, and every
/// numeric kind's raw byte width at the boundary values most likely to expose a truncated
/// read/write.
#[test]
fn dynamic_sample_numeric_and_string_edge_cases() {
    let participant = DdsParticipant::create(Some(63), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::EdgeCases")
        .member("b", DynamicKind::Bool)
        .member("i8", DynamicKind::I8)
        .member("u8", DynamicKind::U8)
        .member("i16", DynamicKind::I16)
        .member("u16", DynamicKind::U16)
        .member("i32", DynamicKind::I32)
        .member("u32", DynamicKind::U32)
        .member("i64", DynamicKind::I64)
        .member("u64", DynamicKind::U64)
        .member("f32", DynamicKind::F32)
        .member("f64", DynamicKind::F64)
        .member("empty_str", DynamicKind::String)
        .member("long_str", DynamicKind::String)
        .build(&participant)
        .expect("build dynamic type");

    let mut sample = DynamicSample::new(&dtype);
    sample.set("b", DynamicValue::Bool(true)).unwrap();
    sample.set("i8", DynamicValue::I8(i8::MIN)).unwrap();
    sample.set("u8", DynamicValue::U8(u8::MAX)).unwrap();
    sample.set("i16", DynamicValue::I16(i16::MIN)).unwrap();
    sample.set("u16", DynamicValue::U16(u16::MAX)).unwrap();
    sample.set("i32", DynamicValue::I32(i32::MIN)).unwrap();
    sample.set("u32", DynamicValue::U32(u32::MAX)).unwrap();
    sample.set("i64", DynamicValue::I64(i64::MIN)).unwrap();
    sample.set("u64", DynamicValue::U64(u64::MAX)).unwrap();
    sample.set("f32", DynamicValue::F32(f32::INFINITY)).unwrap();
    sample
        .set("f64", DynamicValue::F64(f64::NEG_INFINITY))
        .unwrap();
    sample
        .set("empty_str", DynamicValue::String(String::new()))
        .unwrap();
    let long_string: String = "abcdefghij".repeat(2000); // 20,000 chars
    sample
        .set("long_str", DynamicValue::String(long_string.clone()))
        .unwrap();

    assert_eq!(sample.get("b").unwrap(), DynamicValue::Bool(true));
    assert_eq!(sample.get("i8").unwrap(), DynamicValue::I8(i8::MIN));
    assert_eq!(sample.get("u8").unwrap(), DynamicValue::U8(u8::MAX));
    assert_eq!(sample.get("i16").unwrap(), DynamicValue::I16(i16::MIN));
    assert_eq!(sample.get("u16").unwrap(), DynamicValue::U16(u16::MAX));
    assert_eq!(sample.get("i32").unwrap(), DynamicValue::I32(i32::MIN));
    assert_eq!(sample.get("u32").unwrap(), DynamicValue::U32(u32::MAX));
    assert_eq!(sample.get("i64").unwrap(), DynamicValue::I64(i64::MIN));
    assert_eq!(sample.get("u64").unwrap(), DynamicValue::U64(u64::MAX));
    assert_eq!(sample.get("f32").unwrap(), DynamicValue::F32(f32::INFINITY));
    assert_eq!(
        sample.get("f64").unwrap(),
        DynamicValue::F64(f64::NEG_INFINITY)
    );
    assert_eq!(
        sample.get("empty_str").unwrap(),
        DynamicValue::String(String::new())
    );
    assert_eq!(
        sample.get("long_str").unwrap(),
        DynamicValue::String(long_string)
    );

    // Overwriting a string field must free the old allocation rather than leak it - set it
    // again a few times. (Not directly observable from the test, but this is exactly the
    // code path DynamicSample::set()'s "free whatever was there before" branch exists for;
    // running it repeatedly at least exercises it instead of only ever setting each field
    // once across the whole test suite.)
    for i in 0..5 {
        sample
            .set("long_str", DynamicValue::String(format!("iteration-{}", i)))
            .unwrap();
    }
    assert_eq!(
        sample.get("long_str").unwrap(),
        DynamicValue::String("iteration-4".to_string())
    );
}

/// A single-field struct - the smallest possible flat type, no risk of one field's offset
/// accidentally being right only because a neighboring field's padding happened to line up.
#[test]
fn dynamic_type_single_field_struct() {
    let participant = DdsParticipant::create(Some(64), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::SingleField")
        .member("only", DynamicKind::U64)
        .build(&participant)
        .expect("build dynamic type");

    assert_eq!(dtype.size() as usize, std::mem::size_of::<u64>());
    let mut sample = DynamicSample::new(&dtype);
    sample.set("only", DynamicValue::U64(0xdead_beef_cafe_f00d)).unwrap();
    assert_eq!(
        sample.get("only").unwrap(),
        DynamicValue::U64(0xdead_beef_cafe_f00d)
    );
}

/// Same repr(C) cross-check as `dynamic_layout_matches_repr_c_equivalent`, but with the
/// string field declared *first* instead of last - proving offset correctness isn't an
/// artifact of one particular member order (e.g. every fixed-size field happening to come
/// before the one variable-size-looking one).
#[test]
fn dynamic_layout_matches_repr_c_equivalent_reordered() {
    #[repr(C)]
    struct StaticEquivalentReordered {
        label: *mut std::os::raw::c_char,
        id: u32,
        value: f64,
    }

    let participant = DdsParticipant::create(Some(65), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::ReprCCrossCheckReordered")
        .member("label", DynamicKind::String)
        .member("id", DynamicKind::U32)
        .key("id")
        .member("value", DynamicKind::F64)
        .build(&participant)
        .expect("build dynamic type");

    assert_eq!(
        dtype.size() as usize,
        std::mem::size_of::<StaticEquivalentReordered>()
    );
    assert_eq!(
        dtype.align() as usize,
        std::mem::align_of::<StaticEquivalentReordered>()
    );

    let mut sample = DynamicSample::new(&dtype);
    sample.set("id", DynamicValue::U32(99)).unwrap();
    sample.set("value", DynamicValue::F64(-1.25)).unwrap();
    sample
        .set("label", DynamicValue::String("reordered".into()))
        .unwrap();

    let bytes = sample.as_bytes();
    let id_off = std::mem::offset_of!(StaticEquivalentReordered, id);
    let value_off = std::mem::offset_of!(StaticEquivalentReordered, value);
    let label_off = std::mem::offset_of!(StaticEquivalentReordered, label);

    let id = u32::from_ne_bytes(bytes[id_off..id_off + 4].try_into().unwrap());
    assert_eq!(id, 99);
    let value = f64::from_ne_bytes(bytes[value_off..value_off + 8].try_into().unwrap());
    assert_eq!(value, -1.25);

    let ptr_size = std::mem::size_of::<*mut std::os::raw::c_char>();
    let mut ptr_bytes = [0u8; 8];
    ptr_bytes[..ptr_size].copy_from_slice(&bytes[label_off..label_off + ptr_size]);
    let label_ptr = usize::from_ne_bytes(ptr_bytes) as *const std::os::raw::c_char;
    assert!(!label_ptr.is_null());
    let label = unsafe { std::ffi::CStr::from_ptr(label_ptr) }.to_str().unwrap();
    assert_eq!(label, "reordered");
}

/// A very long string over a real DDS write/take round trip (not just in-memory
/// set()/get()) - unlike a static CDR-based sample, a dynamic type's string field is a fixed
/// pointer-sized slot regardless of content length (the variable part is a separate heap
/// allocation CycloneDDS's own wire serialization handles internally), so this specifically
/// exercises CycloneDDS's own string encoding on the wire rather than anything this crate
/// computes.
#[test]
fn dynamic_type_long_string_round_trip_over_dds() {
    let participant = DdsParticipant::create(Some(66), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::LongStringRoundTrip")
        .member("id", DynamicKind::U32)
        .key("id")
        .member("payload", DynamicKind::String)
        .build(&participant)
        .expect("build dynamic type");

    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");
    let topic = DdsDynamicTopic::create(&participant, &dtype, "long_string_topic", None, None)
        .expect("create topic");
    let mut writer =
        DdsDynamicWriter::create(&publisher, &topic, None, None).expect("create writer");
    let reader = DdsDynamicReader::create(&subscriber, &topic, None, None).expect("create reader");

    let payload: String = "0123456789".repeat(5000); // 50,000 chars
    let mut sample = writer.new_sample();
    sample.set("id", DynamicValue::U32(1)).unwrap();
    sample
        .set("payload", DynamicValue::String(payload.clone()))
        .unwrap();
    writer.write(&sample).expect("write");

    let samples = poll_take(&reader, 1, Duration::from_secs(10));
    assert_eq!(samples.len(), 1, "expected 1 sample");
    match samples[0].get("payload").unwrap() {
        DynamicValue::String(s) => assert_eq!(s, payload),
        other => panic!("wrong kind: {:?}", other),
    }
}

/// Several DdsDynamicWriter entities (one per OS thread) publishing to the same dynamic-type
/// topic concurrently. Unlike the loan-based writer stress tests in tests/stress_test.rs,
/// dynamic-type samples never touch loans or this crate's own registries (serdes.rs isn't
/// involved at all - see the module doc on dds_dynamic_type.rs) - a plain dds_write() per
/// sample - so this is mainly checking that CycloneDDS itself, and DynamicSample's own
/// allocation/free path, hold up under genuine multi-thread contention rather than exercising
/// anything specific to this crate's concurrency-control code.
#[test]
fn dynamic_type_concurrent_writers() {
    let participant = DdsParticipant::create(Some(67), None, None).expect("create participant");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::test::ConcurrentWriters")
        .member("id", DynamicKind::U32)
        .key("id")
        .member("label", DynamicKind::String)
        .build(&participant)
        .expect("build dynamic type");

    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");
    let topic = DdsDynamicTopic::create(&participant, &dtype, "concurrent_writers", None, None)
        .expect("create topic");
    let reader = DdsDynamicReader::create(&subscriber, &topic, None, None).expect("create reader");

    const WRITER_COUNT: u32 = 4;
    const PER_WRITER: u32 = 200;
    const TOTAL: u32 = WRITER_COUNT * PER_WRITER;

    let handles: Vec<_> = (0..WRITER_COUNT)
        .map(|w| {
            let mut writer =
                DdsDynamicWriter::create(&publisher, &topic, None, None).expect("create writer");
            std::thread::spawn(move || {
                let base = w * PER_WRITER;
                for offset in 0..PER_WRITER {
                    let id = base + offset;
                    let mut sample = writer.new_sample();
                    sample.set("id", DynamicValue::U32(id)).expect("set id");
                    sample
                        .set("label", DynamicValue::String(format!("w{}-{}", w, offset)))
                        .expect("set label");
                    writer.write(&sample).expect("write");
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("writer thread panicked");
    }

    let samples = poll_take(&reader, TOTAL as usize, Duration::from_secs(30));
    assert_eq!(
        samples.len(),
        TOTAL as usize,
        "expected {} samples, got {}",
        TOTAL,
        samples.len()
    );

    let mut seen = std::collections::HashSet::new();
    for sample in &samples {
        let id = match sample.get("id").unwrap() {
            DynamicValue::U32(v) => v,
            other => panic!("wrong kind: {:?}", other),
        };
        let label = match sample.get("label").unwrap() {
            DynamicValue::String(s) => s,
            other => panic!("wrong kind: {:?}", other),
        };
        let expected_writer = id / PER_WRITER;
        let expected_offset = id % PER_WRITER;
        assert_eq!(
            label,
            format!("w{}-{}", expected_writer, expected_offset),
            "corrupted label for id {}",
            id
        );
        assert!(seen.insert(id), "duplicate id {}", id);
    }
    assert_eq!(seen.len(), TOTAL as usize);
}
