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

use crate::common::EntityWrapper;
use crate::{dds_listener::DdsListener, dds_participant::DdsParticipant, dds_qos::DdsQos, Entity};

use std::{convert::From, marker::PhantomPinned};
use std::ffi::CString;
use std::marker::PhantomData;

use crate::serdes::{TopicType, SerType};
pub use cyclonedds_sys::{DDSError, DdsEntity,ddsi_sertype};


pub struct DdsTopic<T: Sized + TopicType>(EntityWrapper, PhantomData<T>);

impl<T> DdsTopic<T>
where
    T: std::marker::Sized + TopicType,
{
    pub fn create(
        participant: DdsParticipant,
        name: &str,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        let t = SerType::<T>::new();
        let mut t = SerType::into_sertype(t);
        let tt = &mut t as *mut *mut ddsi_sertype;

        unsafe {
            let strname = CString::new(name).expect("CString::new failed");
            let topic = cyclonedds_sys::dds_create_topic_sertype(participant.entity().entity()
                , strname.as_ptr(), tt , 
                maybe_qos.map_or(std::ptr::null(), |q| q.into()), 
                maybe_listener.map_or(std::ptr::null(), |l| l.into()),
                std::ptr::null_mut());

            if topic >= 0 {
                Ok(DdsTopic(EntityWrapper::new(DdsEntity::new(topic)), PhantomData))
            } else {
                Err(DDSError::from(topic))
            }
        }
    }
    
}

impl<T> Entity for DdsTopic<T>
where
    T: std::marker::Sized + TopicType,
{
    fn entity(&self) -> &DdsEntity {
        &self.0.get()
    }
}

impl<T> Clone for DdsTopic<T> 
where T: std::marker::Sized + TopicType
{
    fn clone(&self) -> Self {
        Self(self.0.clone(),PhantomData)
    }
}


#[cfg(test)]
mod test {
    use std::sync::Arc;
    use crate::{DdsPublisher, DdsWriter};
    use super::*;
    use dds_derive::Topic;
    use serde_derive::{Deserialize, Serialize};
    #[test]
    fn test_topic_creation() {

        #[derive(Default, Deserialize, Serialize, Topic)]
        struct MyTopic {
            #[topic_key]
            a : u32,
            b : u32,
            c: String,
            d : u32,
        }

        let participant = DdsParticipant::create(None,None, None).unwrap();
        let topic =  MyTopic::create_topic(participant.clone(), "my_topic", None, None).unwrap();
        let publisher = DdsPublisher::create(participant.clone(), None, None).expect("Unable to create publisher");
        let mut writer = DdsWriter::create(publisher, topic, None, None).unwrap();

       // MyTopic::create_writer()

        let data = Arc::new(MyTopic {
            a : 1,
            b : 32,
            c : "my_data_sample".to_owned(),
            d : 546,
        });

        writer.write(data).unwrap();

    }

}