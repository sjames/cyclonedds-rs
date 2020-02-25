use bit_field::BitField;
use std::convert::From;

use crate::error::DDSError;
use cyclonedds_sys::*;

use crate::dds_writer::DdsWriter;
pub use cyclonedds_sys::dds_status_id;

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

pub fn dds_set_status_mask<T>(
    writer: DdsWriter<T>,
    status_mask: DdsStatus,
) -> Result<(), DDSError> {
    unsafe {
        let err = cyclonedds_sys::dds_set_status_mask(writer.into(), status_mask.into());

        if err < 0 {
            Err(DDSError::from(err))
        } else {
            Ok(())
        }
    }
}

pub fn dds_get_status_changes<T>(writer: DdsWriter<T>) -> Result<DdsStatus, DDSError> {
    unsafe {
        let mut status = DdsStatus::default();
        let err = cyclonedds_sys::dds_get_status_changes(writer.into(), &mut status.0);

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