//! CycloneDDSの組み込み型データリーダーを扱うモジュール
//!
//! 通常のDataReaderと同じ関数を使いながら、DDSの内部の情報を取り出す実装となっている
//! ここではDataReaderの特殊な実装であるBuiltinDataReaderを実装している
use std::{ffi::CStr, fmt::Debug, marker::PhantomData, sync::Arc};

use cyclonedds_sys::{
    dds_copy_qos, dds_create_qos, dds_create_reader, dds_delete_qos, dds_free, dds_read,
    dds_return_loan, dds_sample_info, dds_take, DDSError, DdsEntity,
};

use crate::{
    futures::{participant_reader_listener, ReaderType},
    DdsListener, DdsParticipant, DdsQos, DdsReadable, Entity, Policy,
};

// QoSのユーザー定義型を使って、内部状態に関するプロパティを読み出すための型
pub struct QoSPropertyRef<'a> {
    pub name: &'a CStr,
    pub value: &'a CStr,
}

impl QoSPropertyRef<'_> {
    /// DDSの組み込みトピックで定義されているプロパティ名
    pub const PROCESS_NAME: &'static CStr =
        cyclonedds_sys::DDS_BUILTIN_TOPIC_PARTICIPANT_PROPERTY_PROCESS_NAME;
    pub const PID: &'static CStr = cyclonedds_sys::DDS_BUILTIN_TOPIC_PARTICIPANT_PROPERTY_PID;
    pub const HOST_NAME: &'static CStr =
        cyclonedds_sys::DDS_BUILTIN_TOPIC_PARTICIPANT_PROPERTY_HOSTNAME;
    pub const NETWORK_ADDRESS: &'static CStr =
        cyclonedds_sys::DDS_BUILTIN_TOPIC_PARTICIPANT_PROPERTY_NETWORKADDRESSES;
}

/// BuiltinDataReaderとしての実装
pub trait BuiltinContainer {
    // 読み出したいデータに対応する型
    type Item;
    // BuiltinTopicの規定のID
    const TOPIC: i32;
}

/// DDSの参加インスタンスの型
///
/// 参加者が増えたことだけがわかり、不在になったことはわからない
pub struct Participants;

impl BuiltinContainer for Participants {
    type Item = cyclonedds_sys::dds_builtintopic_participant;
    const TOPIC: i32 = cyclonedds_sys::BUILTIN_TOPIC_DCPSPARTICIPANT;
}

/// DDSのPublishエンドポイントの型
///
/// QoSの有無で追加or削除が区別できる
pub struct Publications;

impl BuiltinContainer for Publications {
    type Item = cyclonedds_sys::dds_builtintopic_endpoint;
    const TOPIC: i32 = cyclonedds_sys::BUILTIN_TOPIC_DCPSPUBLICATION;
}

/// DDSのSubscribeエンドポイントの型
///
/// QoSの有無で追加or削除が区別できる
pub struct Subscriptions;

impl BuiltinContainer for Subscriptions {
    type Item = cyclonedds_sys::dds_builtintopic_endpoint;
    const TOPIC: i32 = cyclonedds_sys::BUILTIN_TOPIC_DCPSSUBSCRIPTION;
}

/// CycloneDDSを使う場合にほぼ必ず設定される参加者プロパティ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CddsParticipantProperty {
    pub id: uuid::Uuid,
    pub process_name: String,
    pub pid: u32,
    pub host_name: String,
    pub network_addresses: String,
}

/// BuiltinSamplesの1サンプル単位にアクセスする型
pub struct BuiltinSample<'a, T> {
    sample: &'a T,
}

impl BuiltinSample<'_, cyclonedds_sys::dds_builtintopic_participant> {
    /// DDS参加者を識別するGUIDを取得する
    pub fn guid(&self) -> uuid::Uuid {
        crate::dds_participant::parse_guid(&self.sample.key)
    }

    /// 参加者が生存中ならtrueを返す
    pub fn is_alive(&self) -> bool {
        !self.sample.qos.is_null()
    }

    /// 参加者のプロパティをイテレータで取得する
    ///
    /// HostName,PIDが読み出せる
    /// 参加者が離脱した場合はNoneを返す
    pub fn props(&self) -> Option<impl Iterator<Item = QoSPropertyRef<'_>>> {
        if self.sample.qos.is_null() {
            return None;
        }
        unsafe {
            let value = &(*self.sample.qos).property.value;
            Some(
                std::slice::from_raw_parts(value.props, value.n as usize)
                    .iter()
                    .map(|prop| QoSPropertyRef {
                        name: CStr::from_ptr(prop.name),
                        value: CStr::from_ptr(prop.value),
                    }),
            )
        }
    }

    /// 参加者の一般的なプロパティを取得する
    pub fn property(&self) -> Result<Option<CddsParticipantProperty>, std::str::Utf8Error> {
        if self.sample.qos.is_null() {
            return Ok(None);
        }
        let guid = self.guid();
        let mut process_name = None;
        let mut pid = None;
        let mut host_name = None;
        let mut network_addresses = None;
        if let Some(props) = self.props() {
            for prop in props {
                if prop.name == QoSPropertyRef::PROCESS_NAME {
                    process_name = Some(prop.value.to_str()?.to_string());
                } else if prop.name == QoSPropertyRef::PID {
                    pid = Some(prop.value.to_str()?.parse().unwrap_or(0));
                } else if prop.name == QoSPropertyRef::HOST_NAME {
                    host_name = Some(prop.value.to_str()?.to_string());
                } else if prop.name == QoSPropertyRef::NETWORK_ADDRESS {
                    network_addresses = Some(prop.value.to_str()?.to_string());
                }
            }
        }

        let Some(process_name) = process_name else {
            return Ok(None);
        };
        let Some(pid) = pid else {
            return Ok(None);
        };
        let Some(host_name) = host_name else {
            return Ok(None);
        };
        let Some(network_addresses) = network_addresses else {
            return Ok(None);
        };
        Ok(Some(CddsParticipantProperty {
            id: guid,
            process_name,
            pid,
            host_name,
            network_addresses,
        }))
    }
}

impl BuiltinSample<'_, cyclonedds_sys::dds_builtintopic_endpoint> {
    /// DDSエンドポイントを識別するGUIDを取得する
    ///
    /// エンドポイント
    pub fn guid(&self) -> uuid::Uuid {
        crate::dds_participant::parse_guid(&self.sample.key)
    }

    /// エンドポイントを所属する参加者のGUIDを取得する
    pub fn participant_guid(&self) -> uuid::Uuid {
        crate::dds_participant::parse_guid(&self.sample.participant_key)
    }

    /// 離脱したイベントの場合はfalseになる
    pub fn is_alive(&self) -> bool {
        !self.sample.qos.is_null()
    }

    /// オンラインの場合はトピック名が取得できる
    pub fn name(&self) -> Option<&CStr> {
        if self.sample.topic_name.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(self.sample.topic_name) })
    }

    /// オンラインの場合はトピックの型が取得できる
    pub fn type_name(&self) -> Option<&CStr> {
        if self.sample.type_name.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(self.sample.type_name) })
    }

    /// トピックのQoSPolicyを取得する
    pub fn policy(&self) -> Option<Policy> {
        if self.sample.qos.is_null() {
            return None;
        }
        Some(Policy::from(self.sample.qos))
    }

    /// トピックのQoSを取得する
    pub fn qos(&self) -> Option<DdsQos> {
        if self.sample.qos.is_null() {
            return None;
        }
        unsafe {
            let q = dds_create_qos();
            if q.is_null() {
                return None;
            }
            let err: DDSError = dds_copy_qos(q, self.sample.qos).into();
            if let DDSError::DdsOk = err {
                Some(DdsQos::from_ptr(q))
            } else {
                dds_delete_qos(q);
                None
            }
        }
    }
}

/// read/take用の構造体
#[derive(Debug)]
pub struct BuiltinSamples<T>
where
    T: BuiltinContainer,
{
    samples: *mut *mut T::Item,
    info: *mut dds_sample_info,
    len: u32,
    max: u32,
    loaned_from: Option<i32>,
}

unsafe fn dds_alloc<T>(len: usize) -> *mut T {
    unsafe { cyclonedds_sys::dds_alloc(size_of::<T>() * len).cast() }
}

impl<T> BuiltinSamples<T>
where
    T: BuiltinContainer,
{
    /// 取得用のメモリ領域を確保
    pub fn new(len: usize) -> Self {
        unsafe {
            Self {
                samples: dds_alloc(len),
                info: dds_alloc(len),
                len: 0,
                max: len as u32,
                loaned_from: None,
            }
        }
    }

    /// 参加者をイテレータで取得
    pub fn iter(&self) -> impl Iterator<Item = BuiltinSample<'_, T::Item>> + '_ {
        unsafe {
            std::slice::from_raw_parts(self.samples, self.len as usize)
                .iter()
                .map(|&s| BuiltinSample { sample: &*s })
        }
    }

    /// 取得したサンプルの開放
    ///
    /// BuiltinSamplesを使いまわす場合は適宜呼び出すこと
    pub fn clear(&mut self) {
        self.return_loan();
    }

    fn return_loan(&mut self) {
        if self.len == 0 {
            self.loaned_from = None;
            return;
        }

        if let Some(reader) = self.loaned_from {
            unsafe {
                let _ = dds_return_loan(reader, self.samples.cast(), self.len as i32);
            }
        }
        self.len = 0;
        self.loaned_from = None;
    }
}

impl<T> Drop for BuiltinSamples<T>
where
    T: BuiltinContainer,
{
    fn drop(&mut self) {
        self.return_loan();
        unsafe {
            dds_free(self.samples.cast());
            dds_free(self.info.cast());
        }
    }
}

struct ReaderInner<T> {
    entity: DdsEntity,
    // 登録している場合はそのメモリを確保するために保持
    _listener: Option<DdsListener>,
    reader_type: ReaderType,
    _phantom: PhantomData<T>,
}

impl<T> ReaderInner<T>
where
    T: BuiltinContainer,
{
    // readerの作成
    // listenerは事前に作り登録時からメモリ位置が変わらないようにその場で構造体に入れてArcで保持する
    fn create_reader(
        p: &DdsParticipant,
        qos: Option<DdsQos>,
        listener: Option<DdsListener>,
        reader_type: ReaderType,
    ) -> Result<Arc<Self>, DDSError> {
        unsafe {
            let r = dds_create_reader(
                DdsReadable::entity(p).entity(),
                T::TOPIC,
                qos.map_or(std::ptr::null(), Into::into),
                listener.as_ref().map_or(std::ptr::null(), Into::into),
            );

            if r >= 0 {
                Ok(Arc::new(Self {
                    entity: DdsEntity::new(r),
                    _listener: listener,
                    reader_type,
                    _phantom: PhantomData,
                }))
            } else {
                Err(DDSError::from(r))
            }
        }
    }

    fn create_async(p: &DdsParticipant, qos: Option<DdsQos>) -> Result<Arc<Self>, DDSError> {
        let (listener, waker) = participant_reader_listener();
        Self::create_reader(p, qos, Some(listener), waker)
    }
}

/// メタ情報を取得するための構造体
pub struct BuiltinDataReader<T> {
    inner: Arc<ReaderInner<T>>,
}

impl<T> BuiltinDataReader<T>
where
    T: BuiltinContainer,
{
    /// 同期リーダーを作成
    pub fn create(p: &DdsParticipant, qos: Option<DdsQos>) -> Result<Self, DDSError> {
        let inner = ReaderInner::create_reader(p, qos, None, ReaderType::Sync)?;
        Ok(BuiltinDataReader { inner })
    }

    /// 非同期リーダーを作成
    pub fn create_async(p: &DdsParticipant, qos: Option<DdsQos>) -> Result<Self, DDSError> {
        let inner = ReaderInner::create_async(p, qos)?;
        Ok(BuiltinDataReader { inner })
    }

    /// 同期読み出し
    pub fn read_now(&self, c: &mut BuiltinSamples<T>) -> Result<usize, DDSError> {
        Self::readn_from_entity_now(&self.inner.entity, c, false)
    }

    /// 同期取り出し
    pub fn take_now(&self, c: &mut BuiltinSamples<T>) -> Result<usize, DDSError> {
        Self::readn_from_entity_now(&self.inner.entity, c, true)
    }

    /// 読み出しor取り出し
    pub fn readn_from_entity_now(
        entity: &DdsEntity,
        c: &mut BuiltinSamples<T>,
        take: bool,
    ) -> Result<usize, DDSError> {
        c.return_loan();

        let ret = unsafe {
            let len = c.max as usize;
            if take {
                dds_take(
                    entity.entity(),
                    c.samples.cast(),
                    c.info as *mut _,
                    len,
                    len as u32,
                )
            } else {
                dds_read(
                    entity.entity(),
                    c.samples.cast(),
                    c.info as *mut _,
                    len,
                    len as u32,
                )
            }
        };
        match ret {
            ..0 => Err(DDSError::from(ret)),
            0 => Err(DDSError::NoData),
            1.. => {
                c.len = ret as u32;
                c.loaned_from = Some(unsafe { entity.entity() });
                Ok(ret as usize)
            }
        }
    }

    /// 保持しているサンプルを非同期で読み出す
    ///
    /// wakerが設定されていない場合は`Err(ReaderError::ReaderNotAsync)`を返す
    pub async fn read_async(
        &self,
        samples: &mut BuiltinSamples<T>,
    ) -> Result<usize, crate::error::ReaderError> {
        crate::futures::read(&self.inner.reader_type, || {
            Self::readn_from_entity_now(self.entity(), samples, false)
        })
        .await
    }

    /// 保持しているサンプルを非同期で取り出す
    ///
    /// wakerが設定されていない場合はErr(ReaderError::ReaderNotAsync)を返す
    pub async fn take_async(
        &self,
        samples: &mut BuiltinSamples<T>,
    ) -> Result<usize, crate::error::ReaderError> {
        crate::futures::read(&self.inner.reader_type, move || {
            Self::readn_from_entity_now(self.entity(), samples, true)
        })
        .await
    }
}

impl<T> Entity for BuiltinDataReader<T> {
    fn entity(&self) -> &DdsEntity {
        &self.inner.entity
    }
}

impl<T> Drop for BuiltinDataReader<T> {
    fn drop(&mut self) {
        unsafe {
            // Listenerより先にReaderを先にDropしなければ、Listenerのコールバックが先に開放されてSEGVが起きる
            let ret: DDSError = cyclonedds_sys::dds_delete(self.inner.entity.entity()).into();
            if DDSError::DdsOk != ret {
                eprintln!("Ignoring dds_delete failure for BuiltinDataReader");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use cyclonedds_derive::Topic;

    use super::*;
    use crate::*;

    #[derive(Debug, Clone, PartialEq, Topic, Serialize, Deserialize)]
    struct TestDiscoveryTopic {
        a: u32,
        b: String,
    }

    impl Default for TestDiscoveryTopic {
        fn default() -> Self {
            Self {
                a: 1,
                b: "test".to_string(),
            }
        }
    }

    // 他の影響を避けるためにloopbackのみを使う設定
    const CYCLONE_LOOPBACK_CONFIG: &str = r###"<?xml version="1.0" encoding="UTF-8" ?>
    <CycloneDDS xmlns="https://cdds.io/config"
                xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                xsi:schemaLocation="https://cdds.io/config https://raw.githubusercontent.com/eclipse-cyclonedds/cyclonedds/iceoryx/etc/cyclonedds.xsd">
        <Domain id="any">
            <General>
                <Interfaces>
                    <NetworkInterface name="lo" priority="default" />
                </Interfaces>
            </General>
        </Domain>
    </CycloneDDS>"###;

    // builtinのテストは副作用を避けるためそれぞれ独自のドメイン内で行う
    const DOMAIN_TEST_PARTICIPANT_ID: u32 = 6;
    const DOMAIN_TEST_ENDPOINT_ID: u32 = 7;

    // 参加者の検知が期待通りか確認
    #[tokio::test]
    #[test_log::test]
    async fn test_discovery_participant() -> anyhow::Result<()> {
        // Make sure iox-roudi is running
        std::env::set_var("CYCLONEDDS_URI", CYCLONE_LOOPBACK_CONFIG);
        let participant = DdsParticipant::create(Some(DOMAIN_TEST_PARTICIPANT_ID), None, None)?;
        let id = participant.guid();

        let reader_partic = BuiltinDataReader::<Participants>::create_async(&participant, None)?;
        let mut sample_partic = BuiltinSamples::<Participants>::new(20);
        let count = reader_partic.take_async(&mut sample_partic).await?;
        // 自身が見つかる。ただし、別プロセスで実行していたタスクが残っている場合は複数見つかるケースがあるので
        // 自身が含まれていたら良しとする
        assert!(count >= 1);
        let res = sample_partic.iter().find(|p| p.guid() == id);
        let res = res.unwrap();
        assert!(res.is_alive());
        let props: Vec<_> = res.props().unwrap().collect();
        assert!(props
            .iter()
            .any(|prop| prop.name == QoSPropertyRef::HOST_NAME));
        assert!(props.iter().any(|prop| prop.name == QoSPropertyRef::PID));

        // 非同期が期待通り0データを無視して待つことを確認
        sample_partic.clear();
        let res = reader_partic.take_now(&mut sample_partic);
        if res.is_ok() {
            for p in sample_partic.iter() {
                println!("Found participant({:?}): {:?}", id, p.guid());
            }
        }
        assert!(matches!(res, Err(DDSError::NoData)));

        let token = tokio_util::sync::CancellationToken::new();
        let cancel = token.clone();

        // create_taskの参加者を検知する
        let read_task = async {
            let mut sample_partic = BuiltinSamples::<Participants>::new(20);
            let count = reader_partic.take_async(&mut sample_partic).await?;
            assert_eq!(count, 1);
            for p in sample_partic.iter() {
                assert_ne!(p.guid(), id);
                assert!(p.is_alive());
                assert!(p.props().is_some());
            }
            cancel.cancel();
            Ok::<(), anyhow::Error>(())
        };

        // create_taskで参加者を作成
        let create_task = async {
            // 想定通りなら待たなくても動作は変化しないが、read開始をなんとなく待つ。
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let participant = DdsParticipant::create(Some(DOMAIN_TEST_PARTICIPANT_ID), None, None)?;
            // readの受信を待つ
            token.cancelled().await;
            drop(participant);
            Ok::<(), anyhow::Error>(())
        };

        tokio::try_join!(read_task, create_task)?;

        // 不在になった記録が残ることを確認
        sample_partic.clear();
        let res = reader_partic.take_now(&mut sample_partic);
        assert_eq!(res, Ok(1));
        for x in sample_partic.iter() {
            assert_ne!(x.guid(), id);
            assert!(!x.is_alive());
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_discovery_endpoint() -> anyhow::Result<()> {
        std::env::set_var("CYCLONEDDS_URI", CYCLONE_LOOPBACK_CONFIG);
        let participant = DdsParticipant::create(Some(DOMAIN_TEST_ENDPOINT_ID), None, None)?;
        let id = participant.guid();

        // publisherが不在ならNoDataになることを確認
        let reader_partic = BuiltinDataReader::<Publications>::create_async(&participant, None)?;
        let mut sample_partic = BuiltinSamples::<Publications>::new(20);

        std::thread::sleep(std::time::Duration::from_millis(100));
        let res = reader_partic.read_now(&mut sample_partic);
        if res.is_ok() {
            for p in sample_partic.iter() {
                println!(
                    "Found publication endpoint({:?}): {:?} {:?}",
                    id,
                    p.guid(),
                    p.name()
                );
            }
        }
        assert_eq!(res, Err(DDSError::NoData));

        // listener登録して一度も読まずに破棄する。listenerの解放が適切にできているか確認
        // 不適切な場合は後続のwriter追加/削除時にSEGVが起きる
        let reader_partic_drop_check =
            BuiltinDataReader::<Publications>::create_async(&participant, None)?;
        drop(reader_partic_drop_check);

        let token = tokio_util::sync::CancellationToken::new();
        let cancel = token.clone();
        let policy = Policy::create_transient_local(10, None);

        // 参加者が見つかり次第タスクが完了する
        let expect_policy = policy.clone();
        let read_task = async {
            let mut sample_partic = BuiltinSamples::<Publications>::new(20);
            if let Ok(count) = reader_partic.take_async(&mut sample_partic).await {
                assert_eq!(count, 1);
                for p in sample_partic.iter() {
                    assert_eq!(p.participant_guid(), id);
                    assert_ne!(p.guid(), id);
                    assert!(p.is_alive());
                    let policy = p.policy().unwrap();
                    assert_eq!(policy, expect_policy);

                    // TODO: change to edition=2024
                    assert_eq!(
                        p.name(),
                        Some(unsafe {
                            CStr::from_bytes_with_nul_unchecked(
                                b"/dds_builtin/tests/TestDiscoveryTopic\0",
                            )
                        })
                    );
                }
            }
            cancel.cancel();
            Ok::<(), anyhow::Error>(())
        };

        let create_task = async {
            // 想定通りなら待たなくても動作は変化しないが、read開始をなんとなく待つ。
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let topic =
                TestDiscoveryTopic::create_topic(&participant, None, Some(policy.to_qos()), None)?;
            let publisher = DdsPublisher::create(&participant, None, None)?;
            let mut writer = DdsWriter::create(&publisher, topic, None, None)?;
            writer.write(Arc::new(TestDiscoveryTopic::default()))?;
            token.cancelled().await;
            drop(writer);
            drop(publisher);
            Ok::<(), anyhow::Error>(())
        };
        tokio::try_join!(read_task, create_task)?;

        // writerを削除の検知を確認
        if let Ok(count) = reader_partic.take_async(&mut sample_partic).await {
            assert_eq!(count, 1);
            for p in sample_partic.iter() {
                assert_eq!(p.participant_guid(), id);
                assert_ne!(p.guid(), id);
                assert!(!p.is_alive());
            }
            sample_partic.clear();
        }
        Ok(())
    }
}
