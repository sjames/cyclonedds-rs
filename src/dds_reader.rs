/*
    Copyright 2021 Sojan James

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

use cyclonedds_sys::*;
use std::convert::From;
use std::future::Future;
use std::os::raw::c_void;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
//use std::convert::TryInto;

pub use cyclonedds_sys::{DdsDomainId, DdsEntity};

use std::marker::PhantomData;

use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsReadable, Entity};
use crate::serdes::{TopicType, Sample};

enum ReaderType {
    Async(Arc<Mutex<Option<Waker>>>),
    Sync,
}


 struct Inner<T: Sized + TopicType> {
    entity: DdsEntity,
    _listener: Option<DdsListener>,
    reader_type : ReaderType,
    _phantom: PhantomData<T>,
    // The callback closures that can be attached to a reader
}

pub struct DdsReader<T: Sized + TopicType> {
    inner : Arc<Inner<T>>,
}

impl<'a, T> DdsReader<T>
where
    T: Sized + TopicType,
{

    pub fn create(
        entity: &dyn DdsReadable,
        topic: DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
    ) -> Result<Self, DDSError> {
        Self::create_sync_or_async(entity, topic, maybe_qos, maybe_listener, ReaderType::Sync)
    }

    fn create_sync_or_async(
        entity: &dyn DdsReadable,
        topic: DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
        maybe_listener: Option<DdsListener>,
        reader_type : ReaderType,
    ) -> Result<Self, DDSError> {
        unsafe {
            let w = dds_create_reader(
                entity.entity().entity(),
                topic.entity().entity(),
                maybe_qos.map_or(std::ptr::null(), |q| q.into()),
                maybe_listener
                    .as_ref()
                    .map_or(std::ptr::null(), |l| l.into()),
            );

            if w >= 0 {
                Ok(DdsReader {
                    inner : Arc::new(Inner {entity: DdsEntity::new(w),
                        _listener: maybe_listener,
                        reader_type,
                        _phantom: PhantomData,})
                })
            } else {
                Err(DDSError::from(w))
            }
        }
    }

    /// Create an async reader. This constructor must be used if using any of the async functions.
    pub fn create_async(
        entity: &dyn DdsReadable,
        topic: DdsTopic<T>,
        maybe_qos: Option<DdsQos>,
    ) -> Result<Self, DDSError> {

        let waker = Arc::new(Mutex::<Option<Waker>>::new(None));
        let waker_cb = waker.clone();
        
        let listener = DdsListener::new()
            .on_data_available(move|_entity| {
                let mut maybe_waker = waker_cb.lock().unwrap();
                if let Some(waker) = maybe_waker.take() {
                    waker.wake();
                }
            })
            .on_requested_deadline_missed(|entity, status| {
                println!("Deadline missed: Entity:{:?} Status:{:?}", unsafe {entity.entity()}, status);
            })
            .hook();

        match Self::create_sync_or_async(entity, topic, maybe_qos, Some(listener),ReaderType::Async(waker) ) {
            Ok(reader) => {
                Ok(reader)
            },
            Err(e) => Err(e),
        }
        
    }

    // read synchronously
    pub fn read_now(&self,buf: &mut [Sample<T>]) -> Result<usize,DDSError> {
        Self::readn_from_entity_now(self.entity(),buf,false)
    }

    // read synchronously
    pub fn take_now(&self,buf: &mut [Sample<T>]) -> Result<usize,DDSError> {
        Self::readn_from_entity_now(self.entity(),buf,true)
    }

    /// Read multiple samples from the reader synchronously. The buffer for the sampes must be passed in.
    /// On success, returns the number of samples read.
    pub fn readn_from_entity_now(entity: &DdsEntity, buf: &mut [Sample<T>], take: bool) -> Result<usize,DDSError> {
        //let mut info = cyclonedds_sys::dds_sample_info::default();
        let mut info = vec![cyclonedds_sys::dds_sample_info::default();buf.len()];
        let info_ptr = info.as_mut_ptr();

        let mut voidp = buf.as_mut_ptr() as *mut c_void;
        let voidpp = &mut voidp;

        let ret = unsafe {
            if take {
                dds_take(entity.entity(), voidpp ,  info_ptr as *mut _, buf.len() as u64, buf.len() as u32)
            } else {
                dds_read(entity.entity(), voidpp ,  info_ptr as *mut _, buf.len() as u64, buf.len() as u32)
            }
        };
        if ret >= 0 {
            if info[0].valid_data {
                   Ok(ret as usize) 
            } else {
                    Err(DDSError::NoData)
            }
        } else {
                Err(DDSError::OutOfResources)
        } 
    }


    /// Read a sample given a DdsEntity.  This is useful when you want to read data
    /// within a closure.
    pub fn read1_from_entity_now(entity: &DdsEntity,) -> Result<Arc<T>, DDSError> {
        let mut samples = [Sample::<T>::default();1];
        match Self::readn_from_entity_now(entity, &mut samples, false) {
            Ok(1) => {
                Ok(samples[0].get().unwrap())
            },
            Ok(_n) => {
                panic!("Expected only one sample");
            }
            Err(e) => Err(e),
        }
    }

    /// Read one sample from the reader
    pub fn read1_now(&self) -> Result<Arc<T>, DDSError> {
       Self::read1_from_entity_now(self.entity()) 
    }

    // Take one sample from the reader given a DdsEntity
    pub fn take1_from_entity_now(entity: &DdsEntity) -> Result<Arc<T>, DDSError> {
        let mut samples = [Sample::<T>::default();1];
        match Self::readn_from_entity_now(entity, &mut samples, true) {
            Ok(1) => {
                Ok(samples[0].get().unwrap())
            },
            Ok(_n) => {
                panic!("Expected only one sample");
            }
            Err(e) => Err(e),
        }
    }
    
    /// Take one sample from the reader.
    pub fn take1_now(&self) -> Result<Arc<T>, DDSError> {
        let mut samples = [Sample::<T>::default();1];
        match Self::readn_from_entity_now(self.entity(), &mut samples, true) {
            Ok(1) => {
                Ok(samples[0].get().unwrap())
            },
            Ok(_n) => {
                panic!("Expected only one sample");
            }
            Err(e) => Err(e),

        }
    }

    /// Read samples asynchronously. The number of samples actually read is returned.
    pub async fn read(&self, samples : &mut[Sample<T>]) -> Result<usize,DDSError> {
        if let ReaderType::Async(waker) = &self.inner.reader_type {
               let future_sample = SampleArrayFuture::new(self.inner.entity.clone(), waker.clone(),samples ,FutureType::Read);
                future_sample.await
           } else {
            panic!("This reader was not constructed with async constructor");
        }
    }

    /// Get samples asynchronously. The number of samples actually read is returned.
    pub async fn take(&self, samples : &mut[Sample<T>]) -> Result<usize,DDSError> {
        if let ReaderType::Async(waker) = &self.inner.reader_type {
            let future_sample = SampleArrayFuture::new(self.inner.entity.clone(), waker.clone(),samples ,FutureType::Take);
             future_sample.await
        } else {
         panic!("This reader was not constructed with async constructor");
     }
    }

    // Get one sample
    pub async fn read1(&self) -> Result<Arc<T>,DDSError> {
        let mut sample = [Sample::<T>::default()];
        match self.read(&mut sample).await {
            Ok(1) => Ok(sample[0].get().unwrap()),
            Ok(_) => Err(DDSError::NoData),
            Err(e) => Err(e),
        }
    }

    pub async fn take1(&self) -> Result<Arc<T>,DDSError> {
        let mut sample = [Sample::<T>::default()];
        match self.take(&mut sample).await {
            Ok(1) => Ok(sample[0].get().unwrap()),
            Ok(_) => Err(DDSError::NoData),
            Err(e) => Err(e),
        }
    }

    pub fn create_readcondition(
        &'a mut self,
        mask: StateMask,
    ) -> Result<DdsReadCondition<T>, DDSError> {
        DdsReadCondition::create(self, mask)
    }
}

impl<'a, T> Entity for DdsReader<T>
where
    T: std::marker::Sized + TopicType,
{
    fn entity(&self) -> &DdsEntity {
        &self.inner.entity
    }
}

impl<'a, T> Drop for DdsReader<T>
where
    T: Sized + TopicType,
{
    fn drop(&mut self) {
        unsafe {
            //println!("Drop reader:{:?}", self.entity().entity());
            let ret: DDSError = cyclonedds_sys::dds_delete(self.inner.entity.entity()).into();
            if DDSError::DdsOk != ret {
                //panic!("cannot delete Reader: {}", ret);
            } else {
                //println!("Reader dropped");
            }
        }
    }
}
 
pub struct DdsReadCondition<'a, T: Sized + TopicType>(DdsEntity, &'a DdsReader<T>);

impl<'a, T> DdsReadCondition<'a, T>
where
    T: Sized + TopicType,
{
    fn create(reader: &'a DdsReader<T>, mask: StateMask) -> Result<Self, DDSError> {
        unsafe {
            let mask: u32 = *mask;
            let p = cyclonedds_sys::dds_create_readcondition(reader.entity().entity(), mask);
            if p > 0 {
                Ok(DdsReadCondition(DdsEntity::new(p), reader))
            } else {
                Err(DDSError::from(p))
            }
        }
    }
}

impl<'a, T> Entity for DdsReadCondition<'a, T>
where
    T: std::marker::Sized + TopicType,
{
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

enum FutureType {
    Take,
    Read,
}

impl FutureType {
    fn is_take(&self) -> bool {
        match self {
            FutureType::Take => true,
            FutureType::Read => false,
        }
    }
}


struct SampleArrayFuture<'a,T> {
    entity : DdsEntity,
    waker : Arc<Mutex<Option<Waker>>>,
    take_or_read : FutureType,
    buffer : &'a mut [Sample<T>]
}


impl <'a,T>SampleArrayFuture<'a,T> {
    fn new(entity: DdsEntity, waker : Arc<Mutex<Option<Waker>>>, buffer: &'a mut [Sample<T>], ty : FutureType) -> Self {
        Self {
            entity,
            waker,
            take_or_read : ty,
            buffer,
        }
    }
}

impl <'a,T>Future for SampleArrayFuture<'a,T> where T: TopicType {
    type Output = Result<usize,DDSError>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {

        // Lock the waker first in case a callback for read complete happens and we miss it
        // clone to avoid the lifetime problem with self
        let waker = self.waker.clone();
        let mut waker = waker.lock().unwrap();
        let is_take = self.take_or_read.is_take();
        let entity = self.entity.clone();

        match DdsReader::<T>::readn_from_entity_now(&entity, self.buffer, is_take) {
            Ok(len) =>  return Poll::Ready(Ok(len)),
            Err(DDSError::NoData) | Err(DDSError::OutOfResources) => {
                let _ = waker.replace(ctx.waker().clone()); 
                Poll::Pending
            },
            Err(e) => {    
                //println!("Error:{}",e);
                // Some other error happened
                Poll::Ready(Err(e))
            }
        }
    }
}



#[cfg(test)]
mod test {
    use core::panic;

    use crate::{DdsParticipant, DdsSubscriber};
    use super::*;
    use crate::{DdsPublisher, DdsWriter};
    use super::*;
    use dds_derive::Topic;
    use serde_derive::{Deserialize, Serialize};
    use tokio::runtime::Runtime;


    #[repr(C)]
    #[derive(Serialize,Deserialize,Debug, PartialEq, Clone)]
    enum Position {
        Front,
        Back,
    }

    impl Default for Position {
        fn default() -> Self {
            Self::Front
        }
    }

    #[derive(Serialize,Deserialize,Topic, Debug, PartialEq)]
    struct TestTopic {
        a : u32,
        b : u16,
        c: String,
        d : Vec<u8>,
        #[topic_key]
        e : u32,
        #[topic_key_enum]
        pos : Position,
    }

    impl Default for TestTopic {
        fn default() -> Self {
            Self {
                a : 10,
                b : 20,
                c : "TestTopic".to_owned(),
                d : vec![1,2,3,4,5],
                e : 0,
                pos : Position::default(),
            }
        }
    }

    #[derive(Serialize,Deserialize,Topic, Debug, PartialEq)]
    struct AnotherTopic {
        pub value : u32,
        pub name : String,
        pub arr : [String;2],
        pub vec : Vec<String>,
        #[topic_key]
        pub key : u32,
    }

    impl Default for AnotherTopic {
        fn default() -> Self {
            assert!(Self::has_key());
            Self {
                value : 42,
                name : "the answer".to_owned(),
                arr : ["one".to_owned(), "two".to_owned()],
                vec : vec!["Hello".to_owned(), "world".to_owned()],
                key : 0,
            }
    }
    }

   

    #[test]
    fn test_reader_async() {
        
        let participant = DdsParticipant::create(None, None, None).unwrap();

        let topic = TestTopic::create_topic(&participant, Some("test_topic"), None, None).unwrap();
        let another_topic = AnotherTopic::create_topic(&participant, None, None, None).unwrap();

        let publisher = DdsPublisher::create(&participant, None, None).unwrap();

        let mut writer = DdsWriter::create(&publisher, topic.clone(), None, None).unwrap();
        let mut another_writer = DdsWriter::create(&publisher, another_topic.clone(), None, None).unwrap();


        let subscriber = DdsSubscriber::create(&participant, None, None).unwrap();
        let reader = DdsReader::create_async(&subscriber, topic.clone(), None).unwrap();
        let another_reader = DdsReader::create_async(&subscriber, another_topic, None).unwrap();

        let rt = Runtime::new().unwrap();

        let _result = rt.block_on(async {
            
            let task = tokio::spawn(async move {
                if let Ok(t) = reader.take1().await {
                    assert_eq!(t,Arc::new(TestTopic::default()));
                } else {
                    panic!("reader get failed");
                }
            });

            let another_task = tokio::spawn(async move {

                let mut two_samples = [Sample::<AnotherTopic>::default(),Sample::<AnotherTopic>::default() ];
                if let Ok(t) = another_reader.read(&mut two_samples).await {
                    assert_eq!(t,1);
                    let samples = &two_samples[..t];
                    assert_eq!(samples[0].get().unwrap(), Arc::new(AnotherTopic::default()));
                } else {
                    panic!("reader get failed");
                }

                

             
            });

            // add a delay to make sure the data is not ready immediately
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let data = Arc::new(TestTopic::default());
            writer.write(data).unwrap();

            another_writer.write(Arc::new(AnotherTopic::default())).unwrap();


            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        });


        

    }

    

}