pub use cyclonedds_sys::DdsEntity;
use std::convert::From;

pub struct DdsSubscriber(DdsEntity);

impl From<DdsSubscriber> for DdsEntity {
    fn from(subscriber: DdsSubscriber) -> Self {
        subscriber.0
    }
}
impl From<&DdsSubscriber> for DdsEntity {
    fn from(subscriber: &DdsSubscriber) -> Self {
        subscriber.0
    }
}
