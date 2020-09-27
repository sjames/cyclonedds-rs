use cyclonedds_rs::{
    self, dds_api, dds_topic::DdsTopic, DdsListener, DdsQos, DdsReader, DdsStatus, DdsWriter,
    Entity,
};

use helloworld_data;

use std::ffi::{CStr, CString};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};

/// Simple hello world test. Sending and receiving one message
#[test]
fn hello_world_idl_test() {
    let receiver = std::thread::spawn(|| subscriber());

    let message_string = CString::new("Hello from DDS Cyclone Rust")
        .expect("Unable to create CString")
        .into_raw();

    let participant = cyclonedds_rs::DdsParticipant::create(None, None, None).unwrap();

    let topic: DdsTopic<helloworld_data::HelloWorldData::Msg> =
        DdsTopic::create(&participant, "HelloWorldData_Msg", None, None)
            .expect("Unable to create topic");

    let mut qos = DdsQos::create().unwrap();
    qos.set_history(
        cyclonedds_rs::dds_qos::dds_history_kind::DDS_HISTORY_KEEP_LAST,
        1,
    );
    qos.set_resource_limits(10, 1, 10);

    let mut writer = DdsWriter::create(&participant, &topic, Some(&qos), None).unwrap();

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

    let msg = helloworld_data::HelloWorldData::Msg {
        userID: 1,
        message: message_string,
    };
    println!("Writing: {}", msg.userID);
    writer.write(&msg).unwrap();

    receiver.join().unwrap();
}

fn subscriber() {
    let participant = cyclonedds_rs::DdsParticipant::create(None, None, None).unwrap();
    let topic: DdsTopic<helloworld_data::HelloWorldData::Msg> =
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

            // cyclonedds_sys::read is unsafe.
            unsafe {
                if let Ok(msg) =
                    cyclonedds_sys::read::<helloworld_data::HelloWorldData::Msg>(&entity)
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
                    tx.send(msg[0].userID).unwrap();
                } else {
                    println!("Error reading");
                }
            }
        })
        .hook();

    if let Ok(mut reader) = DdsReader::create(&participant, &topic, Some(&qos), None) {
        reader
            .set_listener(listener)
            .expect("Unable to set listener");

        let id = rx.recv().unwrap();
        println!("Received :{} completed", id);
    } else {
        panic!("Unable to create reader");
    };
}
