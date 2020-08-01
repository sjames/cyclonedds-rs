use std::convert::From;

use cyclonedds_sys::DdsEntity;
pub use either::Either;

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
