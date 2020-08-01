pub mod alloc;
pub mod dds_api;
pub mod dds_domain;
pub mod dds_listener;
pub mod dds_participant;
pub mod dds_publisher;
pub mod dds_qos;
pub mod dds_reader;
pub mod dds_subscriber;
pub mod dds_topic;
pub mod dds_writer;
pub mod error;

pub use dds_api::*;
pub use dds_listener::DdsListener;
pub use dds_participant::DdsParticipant;
pub use dds_reader::DdsReader;
pub use dds_topic::DdsTopic;
pub use dds_writer::DdsWriter;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
