//extern crate cyclonedds_sys;

use cyclonedds_sys::*;

struct DdsParticipant(dds_entity_t);

impl DdsParticipant {
    pub fn new() -> Self {
        unsafe {
            let p = cyclonedds_sys::dds_create_participant(0, std::ptr::null(), std::ptr::null());
            DdsParticipant(p)
        }
    }
}
