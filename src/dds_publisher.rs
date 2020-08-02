use crate::DdsWritable;
use cyclonedds_sys::DdsEntity;
use std::convert::From;

pub struct DdsPublisher(DdsEntity);

impl From<DdsPublisher> for DdsEntity {
    fn from(publisher: DdsPublisher) -> Self {
        publisher.0
    }
}
impl From<&DdsPublisher> for DdsEntity {
    fn from(publisher: &DdsPublisher) -> Self {
        publisher.0
    }
}

impl DdsWritable for DdsPublisher {
    fn entity(&self) -> DdsEntity {
        self.into()
    }
}
