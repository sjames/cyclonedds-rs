// A guided tour of the Dynamic Type API (docs/design/dynamic-types.md): define a topic
// type's fields at runtime instead of generating a Rust struct at compile time via
// cdds_derive, then publish and subscribe samples by field name.
//
// Run with:
//   LD_LIBRARY_PATH=/usr/local/lib cargo run --example dynamic_types_demo
//
// Everything here runs in one process on one participant for simplicity - see
// examples/dynamic_cross_writer.rs/dynamic_cross_reader.rs for the same idea split across two
// real OS processes talking over the network.

use cyclonedds_rs::*;
use std::time::{Duration, Instant};

fn section(title: &str) {
    println!("\n== {} ==", title);
}

fn main() {
    let domain_id = 90;
    let participant =
        DdsParticipant::create(Some(domain_id), None, None).expect("create participant");

    // --- 1. Define a type at runtime -----------------------------------------------------
    //
    // No struct, no derive macro: SensorReading's shape is only known once this code runs.
    // A real use for this is a generic bridge/inspector that receives its schema from
    // somewhere else at runtime (a config file, a discovery message, ...) rather than a
    // recompile.
    section("Defining a type at runtime");
    let dtype = DynamicTypeBuilder::new_struct("cyclonedds_rs::demo::SensorReading")
        .member("sensor_id", DynamicKind::U32)
        .key("sensor_id")
        .member("temperature_c", DynamicKind::F64)
        .member("label", DynamicKind::String)
        .build(&participant)
        .expect("build dynamic type");

    println!("  type name: {}", dtype.type_name());
    println!(
        "  in-memory layout: {} bytes, {}-byte aligned",
        dtype.size(),
        dtype.align()
    );
    println!("  sensor_id is a key field: {}", dtype.is_key("sensor_id"));
    println!(
        "  temperature_c is a key field: {}",
        dtype.is_key("temperature_c")
    );

    // --- 2. Create a topic, writer, and reader from it ------------------------------------
    //
    // From here on this looks like any other DDS topic - dds_create_topic underneath, the
    // same as a cdds_derive-generated type would use, just built from a descriptor
    // CycloneDDS synthesized from `dtype` instead of one idlc generated at compile time.
    section("Creating topic, writer, and reader");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");
    let topic = DdsDynamicTopic::create(&participant, &dtype, "sensor_readings", None, None)
        .expect("create topic");
    let mut writer =
        DdsDynamicWriter::create(&publisher, &topic, None, None).expect("create writer");
    let reader = DdsDynamicReader::create(&subscriber, &topic, None, None).expect("create reader");
    println!("  done");

    // --- 3. Publish samples by field name --------------------------------------------------
    section("Publishing samples");
    let readings = [
        (1u32, 21.5, "kitchen"),
        (2u32, 18.0, "garage"),
        (3u32, 24.25, "greenhouse"),
    ];
    for (id, temp, label) in readings {
        let mut sample = writer.new_sample();
        sample
            .set("sensor_id", DynamicValue::U32(id))
            .expect("set sensor_id");
        sample
            .set("temperature_c", DynamicValue::F64(temp))
            .expect("set temperature_c");
        sample
            .set("label", DynamicValue::String(label.to_string()))
            .expect("set label");
        writer.write(&sample).expect("write");
        println!("  wrote sensor_id={} temperature_c={} label={:?}", id, temp, label);
    }

    // --- 4. Read them back by field name ----------------------------------------------------
    section("Reading samples back");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while got.len() < readings.len() && Instant::now() < deadline {
        match reader.take(32) {
            Ok(mut samples) => got.append(&mut samples),
            Err(DDSError::NoData) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("take() failed: {:?}", e),
        }
    }
    assert_eq!(got.len(), readings.len(), "didn't receive all samples in time");

    for sample in &got {
        let id = match sample.get("sensor_id").unwrap() {
            DynamicValue::U32(v) => v,
            other => panic!("unexpected kind: {:?}", other),
        };
        let temp = match sample.get("temperature_c").unwrap() {
            DynamicValue::F64(v) => v,
            other => panic!("unexpected kind: {:?}", other),
        };
        let label = match sample.get("label").unwrap() {
            DynamicValue::String(s) => s,
            other => panic!("unexpected kind: {:?}", other),
        };
        println!(
            "  received sensor_id={} temperature_c={} label={:?}",
            id, temp, label
        );
    }

    // --- 5. Kind-checked field access --------------------------------------------------------
    //
    // set()/get() are checked against the type's actual layout - a wrong field name or a
    // value of the wrong kind for that field is a normal Result error, not a panic or (worse)
    // silently writing into the wrong bytes.
    section("Kind checking");
    let mut sample = writer.new_sample();
    match sample.set("temperature_c", DynamicValue::String("oops".into())) {
        Ok(()) => println!("  (unexpected: this should have failed)"),
        Err(e) => println!("  setting temperature_c to a String correctly failed: {:?}", e),
    }
    match sample.set("no_such_field", DynamicValue::U32(0)) {
        Ok(()) => println!("  (unexpected: this should have failed)"),
        Err(e) => println!("  setting an unknown field correctly failed: {:?}", e),
    }

    println!("\nDone.");
}
