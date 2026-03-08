use std::{
    ffi::{c_void, CString},
    sync::Arc,
    thread::{self, sleep},
    time::Duration,
};

use cyclonedds_rs::*;

use cyclonedds_sys::*;

fn main() {
    println!("Subscribing to internal topics");

    let participant = DdsParticipant::create(None, None, None).unwrap();

    unsafe {
        

        
        let mut samples = [std::ptr::null_mut() as *mut dds_builtintopic_endpoint; 2];
        let mut info = [dds_sample_info_t::default(); 2];
        //assert!(reader > 0);

        let listener = DdsListener::new().on_data_available(move |entity|{

            println!("Callback received");

            let ret = dds_take(
                entity.entity(),
                samples.as_mut_ptr() as *mut *mut c_void,
                info.as_mut_ptr(),
                2,
                2,
            );

            if ret > 0 {
                let read_samples = &samples[..ret as usize];
                for sample in read_samples {
                    let sample = **sample;

                    if !sample.topic_name.is_null() && !sample.type_name.is_null() {
                        let topic_name = CString::from_raw(sample.topic_name);
                        let type_name = CString::from_raw(sample.type_name);
                        println!("Topic:{:?}  Type:{:?}", topic_name, type_name);
                        let leak = topic_name.into_raw();
                        let leak = type_name.into_raw();
                    }

                }
                dds_return_loan(entity.entity(), samples.as_mut_ptr() as *mut *mut c_void, ret);
            }
        }).hook();

        let listener_ptr : *const dds_listener_t = (&listener).into();

        let reader = dds_create_reader(
            cyclonedds_rs::DdsReadable::entity(&participant).entity(),
            cyclonedds_sys::builtin_entity::BUILTIN_TOPIC_DCPSPUBLICATION_ENTITY.entity(),
            std::ptr::null_mut(),
            listener_ptr,
        );


        println!("Reader is: {}", reader);


        loop {
            sleep(Duration::from_millis(1000));
        }
    }
}
