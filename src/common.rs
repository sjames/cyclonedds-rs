use cyclonedds_sys::DdsEntity;

/// An entity on which you can attach a DdsWriter
pub trait DdsWritable {
    fn entity(&self) -> DdsEntity;
}

/// An entity on which you can attach a DdsReader
pub trait DdsReadable {
    fn entity(&self) -> DdsEntity;
}
