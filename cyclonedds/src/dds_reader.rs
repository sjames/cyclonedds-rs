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
use std::marker::PhantomData;
use std::sync::Arc;
use tracing::error;

pub use cyclonedds_sys::{DdsDomainId, DdsEntity};

use crate::futures::{data_reader_listener, ReaderType};
use crate::serdes::{SampleBuffer, TopicType};
use crate::{dds_listener::DdsListener, dds_qos::DdsQos, dds_topic::DdsTopic, DdsReadable, Entity};

/// Builder structure for reader
pub struct ReaderBuilder<T: TopicType> {
    maybe_qos: Option<DdsQos>,
    maybe_listener: Option<DdsListener>,
    is_async: bool,
    phantom: PhantomData<T>,
}

impl<T> Default for ReaderBuilder<T>
where
    T: TopicType,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ReaderBuilder<T>
where
    T: TopicType,
{
    pub fn new() -> Self {
        Self {
            maybe_qos: None,
            maybe_listener: None,
            is_async: false,
            phantom: PhantomData,
        }
    }
    /// Create a reader with async support.  If this is enabled,
    /// the builder creates listeners internally. Any listener
    /// passed separately via the `with_listener` api will be
    /// ignored.
    pub fn as_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Create a reader with the specified Qos
    pub fn with_qos(mut self, qos: DdsQos) -> Self {
        self.maybe_qos = Some(qos);
        self
    }

    /// Created a reader with the specified listener.
    /// Note that this is ignored if an async reader
    /// is created.
    pub fn with_listener(mut self, listener: DdsListener) -> Self {
        self.maybe_listener = Some(listener);
        self
    }

    pub fn create(
        self,
        entity: &dyn DdsReadable,
        topic: DdsTopic<T>,
    ) -> Result<DdsReader<T>, DDSError> {
        if self.is_async {
            DdsReader::create_async(entity, topic, self.maybe_qos)
        } else {
            DdsReader::create_sync_or_async(
                entity,
                topic,
                self.maybe_qos,
                self.maybe_listener,
                ReaderType::Sync,
            )
        }
    }
}

struct Inner<T> {
    entity: DdsEntity,
    _listener: Option<DdsListener>,
    reader_type: ReaderType,
    _phantom: PhantomData<T>,
}

impl<T> Inner<T> {
    fn new(
        entity: DdsEntity,
        maybe_listener: Option<DdsListener>,
        reader_type: ReaderType,
    ) -> Self {
        Inner {
            entity,
            _listener: maybe_listener,
            reader_type,
            _phantom: PhantomData,
        }
    }
}

pub struct DdsReader<T> {
    inner: Arc<Inner<T>>,
}

impl<T> DdsReader<T> {
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
        reader_type: ReaderType,
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
                    inner: Arc::new(Inner::new(DdsEntity::new(w), maybe_listener, reader_type)),
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
        let (listener, waker) = data_reader_listener();

        match Self::create_sync_or_async(entity, topic, maybe_qos, Some(listener), waker) {
            Ok(reader) => Ok(reader),
            Err(e) => Err(e),
        }
    }

    /// CDRバッファ(デシリアライズ前)を読み出す
    pub fn readcdrn_from_entity_now(
        entity: &DdsEntity,
        buf: &mut SampleBuffer<T>,
        take: bool,
    ) -> Result<usize, DDSError> {
        use cyclonedds_sys::{
            dds_readcdr, dds_takecdr, DDS_ALIVE_INSTANCE_STATE, DDS_ANY_SAMPLE_STATE,
            DDS_ANY_VIEW_STATE, DDS_NOT_READ_SAMPLE_STATE,
        };
        let maxs = buf.capacity();
        // dds_readcdr/dds_takecdrの場合は内部で`to_sample`が呼び出されないため、SerDataで受信を行う
        let mut data = Box::<[*mut ddsi_serdata]>::new_uninit_slice(maxs);
        let data_ptr = data.as_mut_ptr().cast();
        let info_ptr = buf.sample_info.as_mut_ptr();
        let ret = unsafe {
            if take {
                // 保持しているサンプルのうち、まだ読んでいないものを読む
                let mask =
                    DDS_NOT_READ_SAMPLE_STATE | DDS_ANY_VIEW_STATE | DDS_ALIVE_INSTANCE_STATE;
                dds_takecdr(entity.entity(), data_ptr, maxs as u32, info_ptr, mask)
            } else {
                // 保持しているサンプルすべてを読む
                let mask = DDS_ANY_SAMPLE_STATE | DDS_ANY_VIEW_STATE | DDS_ALIVE_INSTANCE_STATE;
                dds_readcdr(entity.entity(), data_ptr, maxs as u32, info_ptr, mask)
            }
        };
        match ret {
            ..0 => Err(DDSError::from(ret)),
            0 => Err(DDSError::NoData),
            1.. => {
                // 受信データを公開構造体であるSampleBufferにセットする
                for (i, data) in data.iter().enumerate().take(ret as usize) {
                    unsafe {
                        let serdata = data.assume_init();
                        buf.buffer[i].set_serdata(serdata);
                    }
                }
                buf.size = ret as usize;
                Ok(ret as usize)
            }
        }
    }

    /// CDRバッファ(デシリアライズ前)を同期で読み出す。データを消費しないので2回目でも同じデータが得られる
    pub fn readcdr_now(&self, buf: &mut SampleBuffer<T>) -> Result<usize, DDSError> {
        Self::readcdrn_from_entity_now(self.entity(), buf, false)
    }

    /// CDRバッファ(デシリアライズ前)を同期で取り出す
    pub fn takecdr_now(&self, buf: &mut SampleBuffer<T>) -> Result<usize, DDSError> {
        Self::readcdrn_from_entity_now(self.entity(), buf, true)
    }

    /// 保持しているサンプルを非同期で読み出す
    ///
    /// wakerが設定されていない場合は`Err(ReaderError::ReaderNotAsync)`を返す
    pub async fn readcdr_async(
        &self,
        samples: &mut SampleBuffer<T>,
    ) -> Result<usize, crate::error::ReaderError> {
        crate::futures::read(&self.inner.reader_type, || {
            Self::readcdrn_from_entity_now(self.entity(), samples, false)
        })
        .await
    }

    /// 保持しているサンプルを非同期で取り出す
    ///
    /// wakerが設定されていない場合はErr(ReaderError::ReaderNotAsync)を返す
    pub async fn takecdr_async(
        &self,
        samples: &mut SampleBuffer<T>,
    ) -> Result<usize, crate::error::ReaderError> {
        crate::futures::read(&self.inner.reader_type, move || {
            Self::readcdrn_from_entity_now(self.entity(), samples, true)
        })
        .await
    }
}

/// 型付きリーダー
impl<'a, T> DdsReader<T>
where
    T: Sized + TopicType,
{
    /// データを同期で読み出す
    pub fn read_now(&self, buf: &mut SampleBuffer<T>) -> Result<usize, DDSError> {
        Self::readn_from_entity_now(self.entity(), buf, false)
    }

    /// データを同期で取り出す
    pub fn take_now(&self, buf: &mut SampleBuffer<T>) -> Result<usize, DDSError> {
        Self::readn_from_entity_now(self.entity(), buf, true)
    }

    /// データを同期で読み出す（内部でデシリアライズされる）
    pub fn readn_from_entity_now(
        entity: &DdsEntity,
        buf: &mut SampleBuffer<T>,
        take: bool,
    ) -> Result<usize, DDSError> {
        let (mut data, info_ptr) = buf.as_mut_recv_ptr();
        let data_ptr = data.as_mut_ptr().cast();

        let ret = unsafe {
            if take {
                dds_take(
                    entity.entity(),
                    data_ptr,
                    info_ptr as *mut _,
                    buf.len(),
                    buf.len() as u32,
                )
            } else {
                dds_read(
                    entity.entity(),
                    data_ptr,
                    info_ptr as *mut _,
                    buf.len(),
                    buf.len() as u32,
                )
            }
        };
        match ret {
            ..0 => Err(DDSError::from(ret)),
            0 => Err(DDSError::NoData),
            1.. => {
                // 先頭が有効データなら受信分は全て有効とみなす
                if buf.is_valid_sample(0) {
                    buf.size = ret as usize;
                    Ok(ret as usize)
                } else {
                    Err(DDSError::NoData)
                }
            }
        }
    }

    pub fn create_readcondition(
        &'a mut self,
        mask: StateMask,
    ) -> Result<DdsReadCondition<'a, T>, DDSError> {
        DdsReadCondition::create(self, mask)
    }

    /// データを非同期で読み出す
    ///
    /// wakerが設定されていない場合は`Err(ReaderError::ReaderNotAsync)`を返す
    pub async fn read_async(
        &self,
        samples: &mut SampleBuffer<T>,
    ) -> Result<usize, crate::error::ReaderError> {
        crate::futures::read(&self.inner.reader_type, || {
            Self::readn_from_entity_now(self.entity(), samples, false)
        })
        .await
    }

    /// データを非同期で取り出す
    ///
    /// wakerが設定されていない場合はErr(ReaderError::ReaderNotAsync)を返す
    pub async fn take_async(
        &self,
        samples: &mut SampleBuffer<T>,
    ) -> Result<usize, crate::error::ReaderError> {
        crate::futures::read(&self.inner.reader_type, move || {
            Self::readn_from_entity_now(self.entity(), samples, true)
        })
        .await
    }
}

impl<T> Entity for DdsReader<T>
where
    T: std::marker::Sized,
{
    fn entity(&self) -> &DdsEntity {
        &self.inner.entity
    }
}

impl<T> Drop for DdsReader<T>
where
    T: Sized,
{
    fn drop(&mut self) {
        unsafe {
            let ret: DDSError = cyclonedds_sys::dds_delete(self.inner.entity.entity()).into();
            if DDSError::DdsOk != ret {
                error!("Ignoring dds_delete failure for DdsReader");
            }
        }
    }
}

#[allow(dead_code)]
pub struct DdsReadCondition<'a, T: Sized>(DdsEntity, &'a DdsReader<T>);

impl<'a, T> DdsReadCondition<'a, T>
where
    T: Sized,
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
    T: std::marker::Sized,
{
    fn entity(&self) -> &DdsEntity {
        &self.0
    }
}

#[cfg(test)]
mod test {
    use core::panic;
    use std::time::Duration;

    use super::*;
    use crate::{DdsParticipant, DdsSubscriber};
    use crate::{DdsPublisher, DdsWriter};

    use cyclonedds_derive::Topic;
    use serde::{Deserialize, Serialize};
    use tokio::runtime::Runtime;

    #[repr(C)]
    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    enum Position {
        Front,
        Back,
    }

    impl Default for Position {
        fn default() -> Self {
            Self::Front
        }
    }

    #[derive(Serialize, Deserialize, Topic, Debug, PartialEq)]
    struct TestTopic {
        a: u32,
        b: u16,
        c: String,
        d: Vec<u8>,
        #[topic_key]
        e: u32,
        #[topic_key_enum]
        pos: Position,
    }

    impl Default for TestTopic {
        fn default() -> Self {
            Self {
                a: 10,
                b: 20,
                c: "TestTopic".to_owned(),
                d: vec![1, 2, 3, 4, 5],
                e: 0,
                pos: Position::default(),
            }
        }
    }

    #[derive(Serialize, Deserialize, Topic, Debug, PartialEq)]
    struct AnotherTopic {
        pub value: u32,
        pub name: String,
        pub arr: [String; 2],
        pub vec: Vec<String>,
        #[topic_key]
        pub key: u32,
    }

    impl Default for AnotherTopic {
        fn default() -> Self {
            assert!(Self::has_key());
            Self {
                value: 42,
                name: "the answer".to_owned(),
                arr: ["one".to_owned(), "two".to_owned()],
                vec: vec!["Hello".to_owned(), "world".to_owned()],
                key: 0,
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
        let mut another_writer =
            DdsWriter::create(&publisher, another_topic.clone(), None, None).unwrap();

        let subscriber = DdsSubscriber::create(&participant, None, None).unwrap();
        let reader = DdsReader::create_async(&subscriber, topic, None).unwrap();
        let another_reader = DdsReader::create_async(&subscriber, another_topic, None).unwrap();

        let rt = Runtime::new().unwrap();

        let _result = rt.block_on(async {
            let _task = tokio::spawn(async move {
                let mut samplebuffer = SampleBuffer::new(1);
                let res = reader.take_async(&mut samplebuffer).await;
                assert_eq!(res, Err(crate::error::ReaderError::ChangeAliveCount(1)));

                if let Ok(_t) = reader.take_async(&mut samplebuffer).await {
                    let (sample, info) = samplebuffer.iter_items().take(1).next().unwrap();
                    assert!(*sample == TestTopic::default());
                    assert!(info.is_valid());
                    assert!(info.source_timestamp() > Duration::from_nanos(0));
                } else {
                    panic!("reader get failed");
                }
            });

            let _another_task = tokio::spawn(async move {
                let mut samples = AnotherTopic::create_sample_buffer(5);
                let res = another_reader.take_async(&mut samples).await;
                assert_eq!(res, Err(crate::error::ReaderError::ChangeAliveCount(1)));
                if let Ok(t) = another_reader.read_async(&mut samples).await {
                    assert_eq!(t, 1);
                    for s in samples.iter() {
                        println!("Got sample {}", s.key);
                    }
                } else {
                    panic!("reader get failed");
                }
            });

            // add a delay to make sure the data is not ready immediately
            tokio::time::sleep(Duration::from_millis(100)).await;
            let data = Arc::new(TestTopic::default());
            writer.write(data).unwrap();

            another_writer
                .write(Arc::new(AnotherTopic::default()))
                .unwrap();

            tokio::time::sleep(Duration::from_millis(300)).await;
        });
    }
}
