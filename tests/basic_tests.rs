// use cyclonedds_rs::{
//     self, dds_api, dds_topic::DdsTopic, DdsListener, DdsQos, DdsReader, DdsStatus, DdsWriter,
//     SampleBuffer,
//     Entity,
// };
use cyclonedds_rs::*;
use cdds_derive::Topic;

use serde_derive::{Deserialize, Serialize};

#[repr(C)]
#[derive(PartialEq, Debug, Serialize, Deserialize, Topic, Clone)]
pub struct HelloWorldData {
    pub userID: i64,
    pub message: String,
}

use std::ffi::{CStr};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
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
        message: message_string.to_string(),
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

    let (tx, rx): (Sender<i32>, Receiver<i32>) = mpsc::channel();

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
            tx.send(0).unwrap();
            // you could call read here, but then you need to use the unsafe read function exported
            // by cyclonedds-sys.
            /*
            // cyclonedds_sys::read is unsafe.
            unsafe {
                if let Ok(msg) =
                    cyclonedds_sys::read::<HelloWorldData>(&entity)
                {
                    let msg = msg.as_slice();
                    println!("Received {} messages", msg.len());

                    println!("Received message : {}", msg[0].userID);
                    assert_eq!(1, msg[0].userID);
                    assert_eq!(
                        CStr::from_ptr(msg[0].message),
                        CStr::from_bytes_with_nul("Hello from DDS Cyclone Rust\0".as_bytes())
                            .unwrap()
                    );
                    tx.send(0).unwrap();
                } else {
                    println!("Error reading");
                }
            }
            */
        })
        .hook();

    if let Ok(mut reader) = DdsReader::create(&participant, topic, Some(qos), Some(listener)) {
        let mut buf: SampleBuffer<HelloWorldData> = SampleBuffer::new(8);
        let id = rx.recv().unwrap();
        let mut got_value = false;
        let mut num_loops = 0;
        while num_loops < 10 {
            num_loops += 1;
            let res = reader.take_now(&mut buf);
            match res {
                Ok(count) => {
                    let mmsg = buf.get(0).get_sample();
                    println!("Received {} messages", count);

                    if mmsg.is_none() {
                        println!("Received message: None");
                    } else {
                        let msg = mmsg.expect("we just checked");
                        println!("Received message : {}", msg.userID);
                        assert_eq!(1, msg.userID);
                        assert_eq!(
                            msg.message,
                            "Hello from DDS Cyclone Rust"
                        );
                        got_value = true;
                        break;
                    }
                },
                Err(e) => {
                    println!("Error reading: {:?}", e);
                    break;
                }
            }
        }
        let ten_millis = std::time::Duration::from_millis(100);
        std::thread::sleep(ten_millis);
        assert_eq!(got_value, true);
    } else {
        panic!("Unable to create reader");
    };
}
