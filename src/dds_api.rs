use bit_field::BitField;
use std::convert::From;

use crate::error::DDSError;
use cyclonedds_sys::*;

use crate::dds_writer::DdsWriter;
pub use cyclonedds_sys::{dds_status_id, DDSBox};

// re-export constants
pub use cyclonedds_sys::dds_status_id_DDS_DATA_AVAILABLE_STATUS_ID as DDS_DATA_AVAILABLE_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_DATA_ON_READERS_STATUS_ID as DDS_DATA_ON_READERS_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_INCONSISTENT_TOPIC_STATUS_ID as DDS_INCONSISTENT_TOPIC_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_LIVELINESS_CHANGED_STATUS_ID as DDS_LIVELINESS_CHANGED_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_LIVELINESS_LOST_STATUS_ID as DDS_LIVELINESS_LOST_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_OFFERED_DEADLINE_MISSED_STATUS_ID as DDS_OFFERED_DEADLINE_MISSED_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_OFFERED_INCOMPATIBLE_QOS_STATUS_ID as DDS_OFFERED_INCOMPATIBLE_QOS_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_PUBLICATION_MATCHED_STATUS_ID as DDS_PUBLICATION_MATCHED_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_REQUESTED_DEADLINE_MISSED_STATUS_ID as DDS_REQUESTED_DEADLINE_MISSED_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_REQUESTED_INCOMPATIBLE_QOS_STATUS_ID as DDS_REQUESTED_INCOMPATIBLE_QOS_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_SAMPLE_LOST_STATUS_ID as DDS_SAMPLE_LOST_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_SAMPLE_REJECTED_STATUS_ID as DDS_SAMPLE_REJECTED_STATUS_ID;
pub use cyclonedds_sys::dds_status_id_DDS_SUBSCRIPTION_MATCHED_STATUS_ID as DDS_SUBSCRIPTION_MATCHED_STATUS_ID;

pub struct DdsStatus(u32);

impl DdsStatus {
    pub fn set(mut self, id: dds_status_id) -> Self {
        self.0.set_bit(id as usize, true);
        self
    }

    pub fn is_set(&self, id: dds_status_id) -> bool {
        self.0.get_bit(id as usize)
    }
}

impl Default for DdsStatus {
    fn default() -> Self {
        DdsStatus(0)
    }
}

impl From<DdsStatus> for u32 {
    fn from(status: DdsStatus) -> Self {
        status.0
    }
}

pub fn dds_set_status_mask(entity : dds_entity_t, status_mask: DdsStatus) -> Result<(), DDSError>
{
    unsafe {
        let err = cyclonedds_sys::dds_set_status_mask(entity, status_mask.into());

        if err < 0 {
            Err(DDSError::from(err))
        } else {
            Ok(())
        }
    }
}

pub fn dds_get_status_changes(entity : dds_entity_t) -> Result<DdsStatus, DDSError>
{
    unsafe {
        let mut status = DdsStatus::default();
        let err = cyclonedds_sys::dds_get_status_changes(entity, &mut status.0);

        if err < 0 {
            Err(DDSError::from(err))
        } else {
            Ok(status)
        }
    }
}

#[cfg(test)]
mod dds_qos_tests {
    use super::*;
    #[test]
    fn test_dds_status() {
        let status = DdsStatus::default();
        assert_eq!(false, status.is_set(0));
        let status = status
            .set(dds_status_id_DDS_INCONSISTENT_TOPIC_STATUS_ID)
            .set(dds_status_id_DDS_OFFERED_DEADLINE_MISSED_STATUS_ID)
            .set(dds_status_id_DDS_SUBSCRIPTION_MATCHED_STATUS_ID);

        assert_eq!(
            true,
            status.is_set(dds_status_id_DDS_INCONSISTENT_TOPIC_STATUS_ID)
        );
        assert_eq!(
            true,
            status.is_set(dds_status_id_DDS_OFFERED_DEADLINE_MISSED_STATUS_ID)
        );
        assert_eq!(
            true,
            status.is_set(dds_status_id_DDS_SUBSCRIPTION_MATCHED_STATUS_ID)
        );
        assert_eq!(
            false,
            status.is_set(dds_status_id_DDS_SAMPLE_REJECTED_STATUS_ID)
        );
    }

}
