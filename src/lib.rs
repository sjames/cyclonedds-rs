/*
    Copyright 2020 Sojan James

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

pub mod alloc;
mod common;
pub mod dds_api;
pub mod dds_domain;
pub mod dds_listener;
pub mod dds_participant;
pub mod dds_publisher;
pub mod dds_qos;
pub mod dds_reader;
pub mod dds_subscriber;
pub mod dds_topic;
mod dds_waitset;
pub mod dds_writer;
pub mod error;

pub use common::{DdsReadable, DdsWritable, Entity};
pub use dds_api::*;
pub use dds_listener::DdsListener;
pub use dds_participant::DdsParticipant;
pub use dds_publisher::DdsPublisher;
pub use dds_qos::*;
pub use dds_reader::{DdsReadCondition, DdsReader};
pub use dds_subscriber::DdsSubscriber;
pub use dds_topic::DdsTopic;
pub use dds_waitset::DdsWaitset;
pub use dds_writer::DdsWriter;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
