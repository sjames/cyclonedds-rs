use cdds_derive::Topic;
use cyclonedds_rs::serdes::SampleStorage;
use cyclonedds_rs::*;

use serde_derive::{Deserialize, Serialize};

#[repr(C)]
#[derive(PartialEq, Debug, Serialize, Deserialize, Topic, Clone)]
pub struct HelloWorldData {
    pub userID: i64,
    // pub message: String,
}

use std::sync::mpsc;
use std::sync::Arc;

/// Simple hello world test. Sending and receiving one message
#[test]
fn hello_world_idl_test() {
    let receiver = std::thread::spawn(|| subscriber());

    let message_string = "Hello from DDS Cyclone Rust";

    let participant = cyclonedds_rs::DdsParticipant::create(None, None, None).unwrap();

    // The topic is typed by the generated types in the IDL crate.
    let topic: DdsTopic<HelloWorldData> =
        DdsTopic::create(&participant, "HelloWorldData_Msg", None, None)
            .expect("Unable to create topic");

    let mut qos = DdsQos::create().unwrap();
    qos.set_history(
        cyclonedds_rs::dds_qos::dds_history_kind::DDS_HISTORY_KEEP_LAST,
        1,
    );
    qos.set_resource_limits(10, 1, 10);

    let mut writer = DdsWriter::create(&participant, topic, Some(qos), None).unwrap();

    let mut count = 0;

    if let Ok(()) = dds_api::dds_set_status_mask(
        writer.entity(),
        DdsStatus::default().set(dds_api::DDS_PUBLICATION_MATCHED_STATUS_ID),
    ) {
        loop {
            count = count + 1;
            if count > 500 {
                panic!("timeout waiting for publication matched")
            }
            if let Ok(status) = dds_api::dds_get_status_changes(writer.entity()) {
                if status.is_set(dds_api::DDS_PUBLICATION_MATCHED_STATUS_ID) {
                    println!("Publication matched");
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(20));
            } else {
                panic!("dds_get_status failed");
            }
        }
    } else {
        panic!("Unable to set status mask");
    }

    let msg = HelloWorldData {
        userID: 1,
        // message: message_string.to_string(),
    };
    println!("Writing: {}", msg.userID);
    writer.write(Arc::new(msg)).unwrap();

    receiver.join().unwrap();
}

fn subscriber() {
    let participant = cyclonedds_rs::DdsParticipant::create(None, None, None).unwrap();
    // The topic is typed by the generated types in the IDL crate.
    let topic: DdsTopic<HelloWorldData> =
        DdsTopic::create(&participant, "HelloWorldData_Msg", None, None)
            .expect("Unable to create topic");

    let (tx, rx) = mpsc::channel::<Option<SampleStorage<HelloWorldData>>>();

    let mut qos = DdsQos::create().unwrap();
    qos.set_history(
        cyclonedds_rs::dds_qos::dds_history_kind::DDS_HISTORY_KEEP_LAST,
        1,
    );

    let listener = DdsListener::new()
        .on_subscription_matched(move |_, _| {
            println!("Subscription matched");
        })
        .on_data_available(move |entity| {
            println!("Data on reader");
            let mut buf: SampleBuffer<HelloWorldData> = SampleBuffer::new(1);
            let res = DdsReader::readn_from_entity_now(&entity, &mut buf, true);
            match res {
                Ok(count) => {
                    let mmsg = buf.get(0).get_sample();
                    println!("Received {} messages", count);
                    tx.send(mmsg).unwrap();
                }
                Err(e) => {
                    println!("Error reading: {:?}", e);
                }
            }
        })
        .hook();

    let _reader = DdsReader::create(&participant, topic, Some(qos), Some(listener)).unwrap();

    let value = rx.recv().unwrap();

    assert!(value.is_some());

    if let Some(msg) = value {
        assert_eq!(msg.userID, 1);
        // assert_eq!(
        //     msg.message,
        //     "Hello from DDS Cyclone Rust"
        // );
    }
}
