//! serdataに関する実装のモジュール
//!
//! serdataはCycloneDDSでサンプルデータを扱う構造体
//! アプリケーションから渡されたサンプルとCycloneDDS内で扱うときや
//! トランスポート層のイベントを処理してデータをアプリケーションが扱うまで保持するときに紐付けられる
//!
//! serdataは参照カウンタのスマートポインタを実装しており
//! 紐付いたデータの開放はserdataのライフサイクルの中で行われる。
//! しかしデシリアライズのタイミングや、他の保持データについては型ごとに実装する余地があるため
//! [SerData]構造体を用意して、必要な処理を追加する
use std::{ffi::c_void, ptr::NonNull};

use cdr::{Bounded, CdrBe, Infinite};
use cyclonedds_sys::{
    ddsi_keyhash, ddsi_serdata, ddsi_serdata_addref, ddsi_serdata_init, ddsi_serdata_kind,
    ddsi_serdata_ops, ddsi_serdata_removeref, ddsi_sertype, ddsrt_md5_append, ddsrt_md5_finish,
    ddsrt_md5_init, ddsrt_md5_state_t, free_iox_chunk, iceoryx_header_from_chunk, iovec, iox_sub_t,
    nn_rdata, nn_rmsg, IOX_CHUNK_CONTAINS_RAW_DATA, SDK_DATA, SDK_KEY,
};
use serde::{de::DeserializeOwned, Serialize};
use tracing::{error, trace, warn};

use crate::{sertype::SerType, util::SGReader, Sample, TopicType};

/// キー値のハッシュを保持する列挙型
/// 仕様はDDSIエンコーディング仕様書に従う
///
/// 9.6.3.3 KeyHash (PID_KEY_HASH)
/// 16バイトのオクテット配列として定義されている。データ型全てCDRカプセル化サイズが128bit未満なら
/// 次の2つのアルゴリズムいずれかを使用してデータから計算する
///
/// - 128bit未満の場合、全てをCDR_BEでエンコードして埋め込む
/// - それ以外の場合、CDR_BEエンコードしたものをMD5でハッシュ化する
#[derive(Debug, PartialEq, Clone, Default)]
pub(crate) enum KeyHash {
    #[default]
    None,
    // このライブラリで計算するKeyHash。先頭にCDR Encodingのキャップが入るので20バイト
    CdrKey([u8; 20]),
    // DDSI仕様(PID_KEY_HASH)に従うKeyHash。16バイト
    RawKey([u8; 16]),
}

impl KeyHash {
    pub fn key_length(&self) -> usize {
        match self {
            KeyHash::CdrKey(k) => k.len(),
            KeyHash::RawKey(k) => k.len(),
            _ => 0,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) enum SampleData<T> {
    #[default]
    Uninitialized,
    SdkKey,
    SdkData(std::sync::Arc<T>),
    ShmData(NonNull<T>),
}

/// CycloneDDSで取り扱うSerData構造体。
///
/// `serdata_ops`を通じてデータの処理を行うため必要な構造体
/// `ddsi_serdata`をReferenceCounterとする共有された構造体である。
///
/// serdataはUDPからのフラグチェーン、他のserdataからのバイト配列、Iceoryxの共有メモリバッファ、アプリケーションのサンプルデータから構築する
/// このトランスポート層、アプリケーション間の相互変換実装と、それに対応するデータ保持の実装を行う
///
/// 特に、通信経路を自動選択するために可能な限り遅延評価することが期待されている。
/// プロセス内ならCDRエンコードなしにサンプルを共有するなどが可能である
///
/// Tには3タイプある
/// 1. CDRエンコーディングが可能な型: 全ての型が対応するが、ネットワークの1パケットサイズを超えるデータ送受信ではコストが高い
/// 2. メモリレイアウトが固定で、シリアライズなしで送受信できる型
///    メモリコピーで送受信できててコストが低いが、メモリレイアウトが同じでなければならない
/// 3. メモリアロケーションするフィールドを持ち、何らかのrepackingが必要な型
///    CDRエンコードコストはかかるが、サイズが増えても転送コストは増えにくい
///
/// 1が全てを内包しているが、同マシン内ならパフォーマンス観点で2,3を選びたい。
/// 実利用上の制約から3に限定することでロギングがしやすいメリットが生まれる。
#[repr(C)]
pub(crate) struct SerData<T> {
    /// CycloneDDSが使用するserdata構造体
    /// この中ではデータハッシュ、操作関数へのポインタ、Iceoryxのチャンクを保持している
    pub serdata: ddsi_serdata,
    /// 送信時のシリアライズ前、受信時のシリアライズ後データを保持するフィールド
    pub sample: SampleData<T>,
    /// 送信時のデシリアライズ結果、受信時のシリアライズ前データを保持するフィールド
    pub cdr: Option<Vec<u8>>,
    /// キーハッシュを保持するフィールド
    pub key_hash: KeyHash,
    _phantom: std::marker::PhantomData<T>,
}

impl<'a, T> SerData<T> {
    // serops内で行うserdataの初期化
    // Topicの情報はsertype、データの種類はkindで指示される
    fn new(sertype: *const ddsi_sertype, kind: ddsi_serdata_kind) -> Box<SerData<T>> {
        Box::<SerData<T>>::new(Self {
            serdata: {
                let mut data = std::mem::MaybeUninit::uninit();
                unsafe {
                    ddsi_serdata_init(data.as_mut_ptr(), sertype, kind);
                    data.assume_init()
                }
            },
            sample: SampleData::default(),
            cdr: None,
            key_hash: KeyHash::default(),
            _phantom: std::marker::PhantomData,
        })
    }
    // 型名を取得する
    pub fn type_name(&self) -> &str {
        if self.serdata.type_.is_null() {
            return "Untyped";
        }
        let sertype = SerType::<T>::const_ref_from_sertype(self.serdata.type_);
        sertype.type_name()
    }

    // seropsで消費せずに参照するための関数
    pub fn const_ref_from_serdata(serdata: *const ddsi_serdata) -> &'a Self {
        let ptr = serdata as *const SerData<T>;
        unsafe { &*ptr }
    }

    // seropsで消費せずに参照するための関数
    pub fn mut_ref_from_serdata(serdata: *const ddsi_serdata) -> &'a mut Self {
        let ptr = serdata as *mut SerData<T>;
        unsafe { &mut *ptr }
    }

    // serdataポインタを取得する
    fn as_ptr(&self) -> *const ddsi_serdata {
        self as *const Self as *const ddsi_serdata
    }

    // serdataポインタからBoxを作成して所有権を取得する
    pub fn from_raw(ptr: *mut ddsi_serdata) -> Box<Self> {
        unsafe { Box::from_raw(ptr as *mut Self) }
    }
}

impl<T> SerData<T>
where
    T: Serialize,
{
    // サンプルをCDRシリアライズしてserdataで保持する
    fn serialize_sample(&mut self, sample: &T, maybe_size: Option<u32>) -> Result<(), cdr::Error> {
        if let Some(size) = maybe_size {
            // Round up allocation to multiple of four
            let size = (size + 3) & !3u32;
            let mut buffer = Vec::<u8>::with_capacity(size as usize);
            cdr::serialize_into::<_, T, _, CdrBe>(&mut buffer, sample, Infinite)?;
            self.cdr = Some(buffer);
        } else {
            self.cdr = Some(cdr::serialize::<T, _, CdrBe>(sample, Infinite)?);
        }
        Ok(())
    }
}

impl<T> Drop for SerData<T> {
    fn drop(&mut self) {
        // Iceoryxチャンクがあれば解放する
        if !self.serdata.iox_chunk.is_null() {
            unsafe {
                let iox_subscriber = self.serdata.iox_subscriber as *mut iox_sub_t;
                let chunk = &mut self.serdata.iox_chunk;
                let chunk = chunk as *mut *mut c_void;
                free_iox_chunk(iox_subscriber, chunk);
            }
        }
    }
}

// 型Tを必要とするが、それになんの制約がない。操作
// プリミティブ型がわかっていれば使える
pub(crate) fn create_serdata_ops_base<T>() -> Box<ddsi_serdata_ops> {
    Box::new(ddsi_serdata_ops {
        eqkey: Some(serdata_eqkey::<T>),
        // トランスポートレイヤーからの受信する
        from_ser: Some(serdata_from_fragchain::<T>),
        from_ser_iov: Some(serdata_from_ser_iov::<T>),
        from_keyhash: Some(serdata_from_keyhash::<T>),

        // serdataのシリアライズを行う
        to_ser_unref: Some(serdata_to_ser_unref::<T>),

        // 不明。untyped参照を作成する
        to_untyped: Some(serdata_to_untyped::<T>),
        untyped_to_sample: Some(untyped_to_sample::<T>),

        // serdataの開放
        free: Some(serdata_free::<T>),

        // デバッグ
        print: Some(serdata_print::<T>),
        get_keyhash: Some(serdata_get_keyhash::<T>),

        // NOTE: 以下の3つの関数は、本来持つべきシリアライズ実装を持たない
        // Untyped Topicでdds_forwardcdr可能にするために関数設定をしている
        // シリアライズ済みのデータを保持しているはずなので、データ転送のみを行う
        get_size: Some(forward_serdata_get_size::<T>),
        to_ser: Some(forward_serdata_to_ser::<T>),
        to_ser_ref: Some(forward_serdata_to_ser_ref::<T>),

        #[cfg(feature = "shm")]
        from_iox_buffer: Some(serdata_from_iox_buffer::<T>),
        ..Default::default()
    })
}

// Tにシリアライズ実装が必要な操作
pub(crate) fn create_serdata_ops_ser<T>() -> Box<ddsi_serdata_ops>
where
    T: serde::Serialize,
{
    let mut ops = create_serdata_ops_base::<T>();
    ops.get_size = Some(serdata_get_size::<T>);
    ops.to_ser = Some(serdata_to_ser::<T>);
    ops.to_ser_ref = Some(serdata_to_ser_ref::<T>);
    if cfg!(feature = "shm") {
        ops.get_sample_size = Some(serdata_get_sample_size::<T>);
    }
    ops
}

// Tにシリアライズとデシリアライズ実装が必要な操作
pub(crate) fn create_serdata_ops_serdes<T>() -> Box<ddsi_serdata_ops>
where
    T: serde::Serialize + DeserializeOwned + TopicType,
{
    let mut ops = create_serdata_ops_ser::<T>();
    // サンプルからserdataを作成する
    ops.from_sample = Some(serdata_from_sample::<T>);
    // serdataからサンプルを作成する
    ops.to_sample = Some(serdata_to_sample::<T>);
    ops
}

// CDRシリアライズされたキー情報からKeyHashを計算してserdataにセットする
// serdata_default_get_keyhash 関数による設定と同じ
fn compute_key_hash<T>(key_cdr: &[u8], serdata: &mut SerData<T>)
where
    T: TopicType,
{
    let mut cdr_key = [0u8; 20];

    if T::force_md5_keyhash() || key_cdr.len() > 16 {
        let mut md5st = ddsrt_md5_state_t::default();
        let md5set = &mut md5st as *mut ddsrt_md5_state_t;
        unsafe {
            ddsrt_md5_init(md5set);
            ddsrt_md5_append(md5set, key_cdr.as_ptr(), key_cdr.len() as u32);
            ddsrt_md5_finish(md5set, cdr_key.as_mut_ptr());
        }
    } else {
        cdr_key[0..key_cdr.len()].copy_from_slice(key_cdr);
    }
    serdata.key_hash = KeyHash::CdrKey(cdr_key)
}

// 別のserdataが参照を取得するトリガー
// CDRシリアライズ済みデータがあるので共有する
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn forward_serdata_to_ser_ref<T>(
    serdata: *const ddsi_serdata,
    offset: usize,
    size: usize,
    iov: *mut iovec,
) -> *mut ddsi_serdata {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let iov = &mut *iov;
    trace!(type_name = serdata.type_name());

    if let Some(cdr) = &serdata.cdr {
        let cdr = if offset < cdr.len() {
            let last = (offset + size).min(cdr.len());
            &cdr[offset..last]
        } else {
            &[]
        };
        iov.iov_base = if cdr.is_empty() {
            std::ptr::null_mut()
        } else {
            cdr.as_ptr() as *mut c_void
        };
        iov.iov_len = cdr.len();
    } else {
        error!("Serialization error (SHM)!");
        return std::ptr::null_mut();
    }
    ddsi_serdata_addref(&serdata.serdata)
}

/// 別のserdataが参照を取得するトリガー
///
/// 他に使うユーザーが初めて見つかるこのタイミングでシリアライズを行う
/// `to_ser`と同じだが、コピーする代わりに、対応する`to_ser_unref`が呼ばれるまで有効な参照を提供する
/// シリアライズの遅延評価が許可されている
/// また、UDP通信のフラグメントサイズ1024サイズ単位ごとにこの関数が呼ばれるため、
/// シリアライズ結果は保持して、offsetで要求された位置のバッファを詰める必要がある
///
/// ## arguments
///
/// offset: コピー開始位置
/// size: コピーサイズ
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_to_ser_ref<T>(
    serdata: *const ddsi_serdata,
    offset: usize,
    size: usize,
    iov: *mut iovec,
) -> *mut ddsi_serdata
where
    T: Serialize,
{
    fn serialize_type<T: Serialize>(sample: &T, maybe_size: Option<u32>) -> Result<Vec<u8>, ()> {
        if let Some(size) = maybe_size {
            // Round up allocation to multiple of four
            let size = (size + 3) & !3u32;
            let mut buffer = Vec::<u8>::with_capacity(size as usize);
            if let Ok(()) = cdr::serialize_into::<_, T, _, CdrBe>(&mut buffer, sample, Infinite) {
                Ok(buffer)
            } else {
                Err(())
            }
        } else if let Ok(data) = cdr::serialize::<T, _, CdrBe>(sample, Infinite) {
            Ok(data)
        } else {
            Err(())
        }
    }

    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let iov = &mut *iov;
    trace!(type_name = serdata.type_name());

    match &serdata.sample {
        SampleData::Uninitialized => panic!("Attempt to serialize uninitialized Sample"),
        SampleData::SdkKey => {
            let (p, len) = match &serdata.key_hash {
                KeyHash::None => (std::ptr::null(), 0),
                KeyHash::CdrKey(k) => (k.as_ptr(), k.len()),
                KeyHash::RawKey(k) => (k.as_ptr(), k.len()),
            };

            iov.iov_base = p as *mut c_void;
            iov.iov_len = len;
        }
        SampleData::SdkData(sample) => {
            if serdata.cdr.is_none() {
                trace!("do serialization for to_ser_ref");
                let res = serdata.serialize_sample(sample.clone().as_ref(), None);
                if let Err(e) = res {
                    error!("Serialization error: {}", e);
                    return std::ptr::null_mut();
                }
            }
            if let Some(cdr) = &serdata.cdr {
                let cdr = if offset < cdr.len() {
                    // 稀に要求サイズが末尾境界を超えるため、参照可能範囲に切り詰める
                    let last = (offset + size).min(cdr.len());
                    &cdr[offset..last]
                } else {
                    &[]
                };
                iov.iov_base = if cdr.is_empty() {
                    std::ptr::null_mut()
                } else {
                    cdr.as_ptr() as *mut c_void
                };
                iov.iov_len = cdr.len();
            } else {
                error!("Serialization error: no CDR data");
                return std::ptr::null_mut();
            }
        }

        SampleData::ShmData(sample) => {
            if serdata.cdr.is_none() {
                trace!("do serialization for to_ser_ref");
                serdata.cdr = serialize_type::<T>(sample.as_ref(), None).ok();
            }
            if let Some(cdr) = &serdata.cdr {
                let cdr = if offset < cdr.len() {
                    let last = (offset + size).min(cdr.len());
                    &cdr[offset..last]
                } else {
                    &[]
                };
                iov.iov_base = if cdr.is_empty() {
                    std::ptr::null_mut()
                } else {
                    cdr.as_ptr() as *mut c_void
                };
                iov.iov_len = cdr.len();
            } else {
                error!("Serialization error (SHM)!");
                return std::ptr::null_mut();
            }
        }
    }
    ddsi_serdata_addref(&serdata.serdata)
}

// 2つのserdataのキー値が等しいかテストをする
// topicとは無関係にデータのキー値が等しいかをテストする
// キーなしデータのデフォルト `serdata_default_eqkey_nokey` では常にtrueを返す
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_eqkey<T>(
    serdata_a: *const ddsi_serdata,
    serdata_b: *const ddsi_serdata,
) -> bool {
    let a = SerData::<T>::const_ref_from_serdata(serdata_a);
    let b = SerData::<T>::const_ref_from_serdata(serdata_b);
    trace!(type_name = a.type_name(), ?a.key_hash, ?b.key_hash);
    a.key_hash == b.key_hash
}

// ネットワーク経由で受信したフラグチェーンからserdataを構築する
// kindはペイロード種類、sizeはシリアル化されたサイズ
// フラグチェーンは重複する可能性があるが、実用上見たことはないとある
// > - fragchains may overlap, though I have never seen any DDS implementation
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_from_fragchain<T>(
    sertype: *const ddsi_sertype,
    kind: ddsi_serdata_kind,
    mut fragchain: *const nn_rdata,
    size: usize,
) -> *mut ddsi_serdata {
    /*  These functions are created from the macros in
        https://github.com/eclipse-cyclonedds/cyclonedds/blob/f879dc0ef56eb00857c0cbb66ee87c577ff527e8/src/core/ddsi/include/dds/ddsi/q_radmin.h#L108
        Bad things will happen if these macros change.
        Some discussions here: https://github.com/eclipse-cyclonedds/cyclonedds/issues/830
    */
    fn nn_rdata_payload_offset(rdata: *const nn_rdata) -> usize {
        unsafe { (*rdata).payload_zoff as usize }
    }

    fn nn_rmsg_payload(rmsg: *const nn_rmsg) -> *const u8 {
        unsafe { rmsg.add(1) as *const u8 }
    }

    fn nn_rmsg_payload_offset(rmsg: *const nn_rmsg, offset: usize) -> *const u8 {
        unsafe { nn_rmsg_payload(rmsg).add(offset) }
    }

    let mut off: u32 = 0;
    let fragchain_ref = &*fragchain;
    let mut serdata = SerData::<T>::new(sertype, kind);
    trace!(type_name = serdata.type_name(), size);

    assert_eq!(fragchain_ref.min, 0);
    assert!(fragchain_ref.maxp1 >= off);

    // The scatter gather list
    let mut sg_list = Vec::new();

    while !fragchain.is_null() {
        let fragchain_ref = &*fragchain;
        if fragchain_ref.maxp1 > off {
            let payload =
                nn_rmsg_payload_offset(fragchain_ref.rmsg, nn_rdata_payload_offset(fragchain));
            let src = payload.add((off - fragchain_ref.min) as usize);
            let n_bytes = fragchain_ref.maxp1 - off;
            sg_list.push(std::slice::from_raw_parts(src, n_bytes as usize));
            off = fragchain_ref.maxp1;
            assert!(off as usize <= size);
        }
        fragchain = fragchain_ref.nextfrag;
    }

    // make a reader out of the sg_list
    let reader = SGReader::new(&sg_list);
    serdata.cdr = Some(
        reader
            .into_vec(size)
            .expect("Failed to read fragchain into vec"),
    );

    let ptr = Box::into_raw(serdata);
    ptr as *mut ddsi_serdata
}

// `from_ser`と全く同じだが、データ重複がないことが保証される
// 主な呼ばれ方としてはwriterのserdataの参照を確保した状態で、受信serdataを構築する場合に呼ばれる
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_from_ser_iov<T>(
    sertype: *const ddsi_sertype,
    kind: ddsi_serdata_kind,
    niov: usize,
    iov: *const iovec,
    size: usize,
) -> *mut ddsi_serdata {
    let mut serdata = SerData::<T>::new(sertype, kind);
    trace!(type_name = serdata.type_name(), serdata = ?serdata.as_ptr(), size, niov);

    let iovs = std::slice::from_raw_parts(iov, niov);
    let iov_slices: Vec<&[u8]> = iovs
        .iter()
        .map(|iov| std::slice::from_raw_parts(iov.iov_base as *const u8, iov.iov_len))
        .collect();

    // make a reader out of the sg_list
    let reader = SGReader::new(&iov_slices);
    serdata.cdr = Some(reader.into_vec(size).expect("Failed to read iov"));

    let ptr = Box::into_raw(serdata);
    ptr as *mut ddsi_serdata
}

// serdataをKeyHashから構築する
// 復元できる場合は復元し、不可能な場合はNullを返す
// ddsi_serdata_from_keyhash_cdrやddsi_serdata_from_keyhash_cdr_nokeyを参考にする
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_from_keyhash<T>(
    sertype: *const ddsi_sertype,
    keyhash: *const ddsi_keyhash,
) -> *mut ddsi_serdata {
    let keyhash = (*keyhash).value;
    let sertype_ = SerType::<T>::const_ref_from_sertype(sertype);
    trace!(type_name = sertype_.type_name(), ?keyhash);

    // SDK_KEYのserdataを生成し、KeyHashをコピーする
    // 実質的には単なるダミーデータを作っているように見える
    let mut serdata = SerData::<T>::new(sertype, SDK_KEY);
    serdata.sample = SampleData::SdkKey;
    let mut key_hash_buffer = [0u8; 20];
    key_hash_buffer[4..].copy_from_slice(&keyhash);
    serdata.key_hash = KeyHash::CdrKey(key_hash_buffer);

    let ptr = Box::into_raw(serdata);
    ptr as *mut ddsi_serdata
}

// クライアントのデータを元にserdataを構築する
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_from_sample<T>(
    sertype: *const ddsi_sertype,
    kind: ddsi_serdata_kind,
    sample: *const c_void,
) -> *mut ddsi_serdata
where
    T: TopicType,
{
    // CycloneDDSで管理するserdataの初期化
    let mut serdata = SerData::<T>::new(sertype, kind);
    trace!(type_name = serdata.type_name(), serdata = ?serdata.as_ptr());

    match kind {
        SDK_DATA => {
            // NOTE: return_loan()で渡されるTのポインタが渡される可能性はあるが、これはサポートしない。
            // loanなのにsampleが呼ばれるケースは、シリアライズなし転送の想定ルートに
            // シリアライズ必要な参加者が入る異常な状況なので許すべきではない
            // 一応区別する場合はheaderが読めるかでどうかが利用できる。
            // let iox_header = iceoryx_header_from_chunk(sample);
            // let mut sample = NonNull::new_unchecked(sample as *mut T);

            // sampleにはSample<T>が書いてある
            // write by [crate::dds_writer::DdsWriter::write_to_entity]
            let sample = sample as *const Sample<T>;
            let sample = &*sample;
            let sample = sample.get_expected();
            serdata.serdata.hash = sample.hash((*sertype).serdata_basehash);
            serdata.sample = SampleData::SdkData(sample);
        }
        SDK_KEY => {
            panic!("Don't know how to create serdata from sample for SDK_KEY");
        }
        _ => panic!("Unexpected kind"),
    }

    // serdataはCycloneDDSが扱うのでRustでの所有権を放棄する
    let ptr = Box::into_raw(serdata);
    ptr as *mut ddsi_serdata
}

fn copy_cdr_data(s: &[u8], size: usize, offset: usize, buf: *mut u8) {
    if size == 0 {
        return;
    }

    if offset >= s.len() {
        unsafe {
            std::ptr::write_bytes(buf, 0, size);
        }
        return;
    }

    let last = (offset + size).min(s.len());
    let cdr = &s[offset..last];
    unsafe {
        std::ptr::copy_nonoverlapping(cdr.as_ptr(), buf, cdr.len());
        if cdr.len() < size {
            std::ptr::write_bytes(buf.add(cdr.len()), 0, size - cdr.len());
        }
    };
}

// offから始まるsizeバイトのシリアル化データでバッファを埋める
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn forward_serdata_to_ser<T>(
    serdata: *const ddsi_serdata,
    size: usize,
    offset: usize,
    buf: *mut c_void,
) {
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    trace!(type_name = serdata.type_name(), size, offset);
    let buf = buf as *mut u8;
    let buf = buf.add(offset);

    if size == 0 {
        return;
    }

    // CDR済みのデータがあるはずなのでコピーのみ行う
    if let Some(ref v) = &serdata.cdr {
        copy_cdr_data(v.as_slice(), size, offset, buf);
    }
}

// offから始まるsizeバイトのシリアル化データでバッファを埋める
// > 0 <= off < off+sz <= alignup4(size(d))
//
// DDSIエンコーディングヘッダを含む
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_to_ser<T>(
    serdata: *const ddsi_serdata,
    size: usize,
    offset: usize,
    buf: *mut c_void,
) where
    T: Serialize,
{
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    trace!(type_name = serdata.type_name(), size, offset);
    let buf = buf as *mut u8;
    let buf = buf.add(offset);

    if size == 0 {
        return;
    }

    // CDR済みのデータがあればそれをコピーする
    if let Some(ref v) = &serdata.cdr {
        copy_cdr_data(v.as_slice(), size, offset, buf);
        return;
    }

    // シリアライズは1度だけ行われるはず?
    trace!(type_name = serdata.type_name(), "Serializing serdata");
    match &serdata.sample {
        SampleData::Uninitialized => {
            panic!("Attempt to serialize uninitialized serdata")
        }
        SampleData::SdkKey => match &serdata.key_hash {
            KeyHash::None => std::ptr::write_bytes(buf, 0, size),
            KeyHash::CdrKey(k) => copy_cdr_data(k, size, offset, buf),
            KeyHash::RawKey(k) => copy_cdr_data(k, size, offset, buf),
        },
        // We may serialize both SDK data as well as SHM Data
        SampleData::SdkData(v) => {
            let buf_slice = std::slice::from_raw_parts_mut(buf, size);
            if let Err(e) =
                cdr::serialize_into::<_, T, _, CdrBe>(buf_slice, v.as_ref(), Bounded(size as u64))
            {
                panic!(
                    "Unable to serialize type {:?} due to {}",
                    serdata.type_name(),
                    e
                );
            }
        }
        SampleData::ShmData(v) => {
            let buf_slice = std::slice::from_raw_parts_mut(buf, size);
            if let Err(e) =
                cdr::serialize_into::<_, T, _, CdrBe>(buf_slice, v.as_ref(), Bounded(size as u64))
            {
                panic!(
                    "Unable to serialize type {:?} due to {}",
                    serdata.type_name(),
                    e
                );
            }
        }
    }
}

// `to_ser_ref`で取得したシリアライズデータのロックを解除する
// iovはserdata.cdrの内容を指しているので解放する必要はない
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_to_ser_unref<T>(serdata: *mut ddsi_serdata, _iov: *const iovec) {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    trace!(type_name = serdata.type_name());
    ddsi_serdata_removeref(&mut serdata.serdata)
}

// Key値のみを持つ型なしserdataを構築する
// tkmap(データの保持テーブル)にある共有されたデータへのアクセスを提供する
// untypedによってはreturn ddsi_serdata_ref(d)するだけで問題ない?
// serdata_default_to_untypedではhashだけコピーしている
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_to_untyped<T>(serdata: *const ddsi_serdata) -> *mut ddsi_serdata {
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    // untyped serdataを作成する
    let mut untyped_serdata = SerData::<T>::new(serdata.serdata.type_, SDK_KEY);
    untyped_serdata.serdata.type_ = std::ptr::null_mut();
    untyped_serdata.sample = SampleData::SdkKey;

    // copy the hashes
    untyped_serdata.key_hash = serdata.key_hash.clone();
    untyped_serdata.serdata.hash = serdata.serdata.hash;
    trace!(type_name = serdata.type_name(), ?serdata.key_hash, data_hash = serdata.serdata.hash);

    let ptr = Box::into_raw(untyped_serdata);
    ptr as *mut ddsi_serdata
}

// `to_untyped`で返される型指定のないserdataからサンプルを作成する。
// キーのみが入力されて、無効なサンプルを生成するために使用される
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn untyped_to_sample<T>(
    _sertype: *const ddsi_sertype,
    serdata: *const ddsi_serdata,
    sample: *mut c_void,
    _buf: *mut *mut c_void,
    _buflim: *mut c_void,
) -> bool {
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    trace!(type_name = serdata.type_name());

    // よくわかっていないが、無効なサンプルをという話があるので中身を開放する
    if !sample.is_null() {
        let sample = Sample::mut_ref_from_sample(sample as *mut Sample<T>);
        sample.free_contents();
        true
    } else {
        false
    }
}

// serdataを解放する。redcount=0になったときに呼ばれる
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_free<T>(serdata: *mut ddsi_serdata) {
    // Rustで所有権を取得し、この関数内で解放する
    let serdata = SerData::<T>::from_raw(serdata);
    trace!(type_name = serdata.type_name());
}

// 指定バッファにserdataを出力する
// topicはuntypedサンプルの出力補助のために存在する
// bufは常にnull文字終端が必要で、出力に必要な文字数(null終端を含除く)を返す
// 最適化のために必要な場合はbufsize-1を返すことで出力したとみなすこともできる
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_print<T>(
    _sertype: *const ddsi_sertype,
    serdata: *const ddsi_serdata,
    _buf: *mut std::os::raw::c_char,
    _bufsize: usize,
) -> usize {
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    trace!(type_name = serdata.type_name());
    0
}

// serdataから取得したキーハッシュをバッファに追加する
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_get_keyhash<T>(
    serdata: *const ddsi_serdata,
    keyhash: *mut ddsi_keyhash,
    _force_md5: bool,
) {
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    let keyhash = &mut *keyhash;

    let src = match &serdata.key_hash {
        KeyHash::None => &[],
        KeyHash::CdrKey(k) => &k[4..],
        KeyHash::RawKey(k) => &k[..],
    };
    if !src.is_empty() {
        keyhash.value.copy_from_slice(src);
    }
}

// データサイズを返す。CDRデータがあるはずなので長さを返す
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn forward_serdata_get_size<T>(serdata: *const ddsi_serdata) -> u32 {
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let size = serdata.cdr.as_ref().map_or(0, |cdr| cdr.len() as u32);
    trace!(type_name = serdata.type_name(), size);
    size
}

// DDSIエンコーディングヘッダを含むサンプルのシリアル化データのサイズを返す
// UDP転送時に必要になる
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_get_size<T>(serdata: *const ddsi_serdata) -> u32
where
    T: Serialize,
{
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
    let size = match &serdata.sample {
        SampleData::SdkKey => serdata.key_hash.key_length() as u32,
        SampleData::SdkData(sample) => {
            if let Some(cdr) = &serdata.cdr {
                return cdr.len() as u32;
            }
            cdr::calc_serialized_size::<T>(sample) as u32
        }
        // SampleData::ShmData(sample) => cdr::calc_serialized_size::<T>(sample.as_ref()) as u32,
        _ => 0,
    };
    trace!(type_name = serdata.type_name(), size);
    size
}

// Iceoryxの共有メモリバッファのサイズを返す
// ddsi_serdata_iox_size
#[cfg(feature = "shm")]
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_get_sample_size<T>(serdata: *const ddsi_serdata) -> u32
where
    T: Serialize,
{
    let serdata = SerData::<T>::const_ref_from_serdata(serdata);
    (*serdata.serdata.type_).iox_size
}

// 受信したserdataからサンプルを復元する
// 受信時点ではデシリアライズは行わず、アプリケーションがサンプルを必要としたときにデシリアライズを行う
// また、一度デシリアライズしたらserdataで保持して、再利用する
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_to_sample<T>(
    serdata_ptr: *const ddsi_serdata,
    sample: *mut ::std::os::raw::c_void,
    _bufptr: *mut *mut ::std::os::raw::c_void,
    _buflim: *mut ::std::os::raw::c_void,
) -> bool
where
    T: DeserializeOwned + TopicType,
{
    let serdata = SerData::<T>::mut_ref_from_serdata(serdata_ptr);
    assert!(!sample.is_null());

    let s = Sample::mut_ref_from_sample(sample as *mut Sample<T>);
    trace!(type_name = serdata.type_name(), sample.serdata = ?s.serdata, serdata.serdata = ?serdata.as_ptr(), refc = ?serdata.serdata.refc);

    // 外部から到来したメッセージで、デシリアライズ未了の場合の分岐
    // デシリアライズしたならserdata.sampleがUninitializedではなくなる
    if let SampleData::Uninitialized = serdata.sample {
        // ioxが有効で、iox_chunkがセットされている場合はiox_chunkからデシリアライズする
        if cfg!(feature = "shm") && !serdata.serdata.iox_chunk.is_null() {
            let iox_header = iceoryx_header_from_chunk(serdata.serdata.iox_chunk);
            let size = (*iox_header).data_size as usize;
            let buf =
                unsafe { std::slice::from_raw_parts(serdata.serdata.iox_chunk as *const u8, size) };

            // ioxサンプルをデシリアライズしたら複数回呼ばれた場合に備えてsampleに記録する
            match cdr::deserialize_from::<_, T, _>(buf, Bounded(size as u64)) {
                Ok(decoded) => {
                    if T::has_key() {
                        // compute the 16byte key hash
                        let key_cdr = decoded.key_cdr();
                        // skip the four byte header
                        let key_cdr = &key_cdr[4..];
                        compute_key_hash(key_cdr, serdata);
                    }
                    let sample = std::sync::Arc::new(decoded);
                    serdata.sample = SampleData::SdkData(sample);
                }
                Err(e) => {
                    warn!(type_name = serdata.type_name(), sample.serdata = ?s.serdata, serdata.serdata = ?serdata.as_ptr(), error = %e,"Deserialization error!");
                    return false;
                }
            }
        } else {
            // ioxが無効もしくは未設定ならCDRにデータがなければならない
            if serdata.cdr.is_none() {
                warn!(type_name = serdata.type_name(), sample.serdata = ?s.serdata, serdata.serdata = ?serdata.as_ptr(),"CDR data is missing!");
                return false;
            }
            if let Some(cdr) = &serdata.cdr {
                let size = cdr.len();
                match cdr::deserialize_from::<_, T, _>(cdr.as_slice(), Bounded(size as u64)) {
                    Ok(decoded) => {
                        let sample = std::sync::Arc::new(decoded);
                        serdata.sample = SampleData::SdkData(sample);
                    }
                    Err(e) => {
                        warn!(type_name = serdata.type_name(), sample.serdata = ?s.serdata, serdata.serdata = ?serdata.as_ptr(), error = %e,"Deserialization error!");
                        return false;
                    }
                }
            }
        }
    }

    // 有効な受信データの場合はサンプルに関連付ける
    match &serdata.sample {
        SampleData::SdkData(_) | SampleData::ShmData(_) => {
            s.set_serdata(serdata_ptr as *mut ddsi_serdata);
        }
        _ => {}
    }

    true
}

// Iceoryxで確保したバッファを紐付けて、アプリケーションが読み出せる状態にする
// iox_chunkにはそのまま、もしくはcdrシリアライズされたデータが入っている
// headerにフラグがあって区別ができる
#[tracing::instrument(level = "trace")]
unsafe extern "C" fn serdata_from_iox_buffer<T>(
    sertype: *const ddsi_sertype,
    kind: ddsi_serdata_kind,
    sub: *mut ::std::os::raw::c_void,
    buffer: *mut ::std::os::raw::c_void,
) -> *mut ddsi_serdata {
    if sertype.is_null() {
        trace!("sertype is null");
        return std::ptr::null_mut();
    }
    let mut d = SerData::<T>::new(sertype, kind);

    // iox_chunkはserdataに渡して管理を任せる
    d.serdata.iox_chunk = buffer;
    let iox_header = iceoryx_header_from_chunk(buffer);
    trace!(type_name = d.type_name(), serdata = ?d.as_ptr(),size = &(*iox_header).data_size, state = &(*iox_header).shm_data_state);

    // サブスクライバがいる場合は紐付けとkey_hashのコピーを行う
    if !sub.is_null() {
        d.serdata.iox_subscriber = sub;
        d.key_hash = KeyHash::RawKey((*iox_header).keyhash.value);
    }

    // シリアライズなしでデータが入っている場合は参照を作る
    if (*iox_header).shm_data_state == IOX_CHUNK_CONTAINS_RAW_DATA {
        d.sample = SampleData::ShmData(NonNull::new_unchecked(buffer as *mut T));
    }

    // serdataのポインタを返す
    let ptr = Box::into_raw(d);
    ptr as *mut ddsi_serdata
}

#[cfg(test)]
mod tests {
    use std::{ffi::c_void, mem::MaybeUninit};

    use cyclonedds_sys::{ddsi_sertype, SDK_DATA};

    use super::*;
    use crate::{
        common::tests::TestTypeAlloc,
        sertype::{
            tests::{IoxChunk, SerTypeOps, Writer},
            SerType,
        },
        DdsParticipant, DdsPublisher, DdsTopic, Sample,
    };

    // serdata_opsの各関数を呼び出すテスト構造体
    struct SerDataOps<'a, T> {
        sertype: &'a SerType<T>,
    }

    impl<'a, T> SerDataOps<'a, T> {
        fn new(sertype: &'a SerType<T>) -> Self {
            Self { sertype }
        }

        #[inline]
        fn ops(&self) -> &cyclonedds_sys::ddsi_serdata_ops {
            unsafe { &*self.sertype.sertype.serdata_ops }
        }

        #[inline]
        unsafe fn sertype_ptr(&self) -> *const ddsi_sertype {
            self.sertype as *const SerType<T> as *const ddsi_sertype
        }

        #[inline]
        unsafe fn sample_ptr(sample: &Sample<T>) -> *const c_void {
            sample as *const Sample<T> as *const c_void
        }

        #[inline]
        unsafe fn sample_mut_ptr(sample: &mut Sample<T>) -> *mut c_void {
            sample as *mut Sample<T> as *mut c_void
        }

        // ops->from_sample
        fn serdata_from_sample(&self, sample: &Sample<T>) -> Box<SerData<T>> {
            let ptr = unsafe {
                self.ops().from_sample.unwrap()(
                    self.sertype_ptr(),
                    SDK_DATA,
                    Self::sample_ptr(sample),
                )
            };
            SerData::from_raw(ptr)
        }

        // ops->to_sampleを呼び出す
        fn to_sample(&self, serdata: &SerData<T>, sample: &mut Sample<T>) -> bool {
            unsafe {
                self.ops().to_sample.unwrap()(
                    serdata.as_ptr(),
                    Self::sample_mut_ptr(sample),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            }
        }

        // ops->get_sizeを呼び出す
        fn get_size(&self, serdata: &SerData<T>) -> u32 {
            unsafe { self.ops().get_size.unwrap()(serdata.as_ptr()) }
        }

        // to_ser_ref -> serdata_from_ser_iov -> to_ser_unrefの一連の操作
        fn serdata_from_ser_iov_seq(&self, serdata: &SerData<T>) -> Box<SerData<T>> {
            unsafe {
                let size = self.get_size(serdata);
                let mut iov: cyclonedds_sys::iovec = MaybeUninit::zeroed().assume_init();
                // copyの間だけ生存する
                let serdata_ref =
                    self.ops().to_ser_ref.unwrap()(serdata.as_ptr(), 0, size as usize, &mut iov);
                let res = self.ops().from_ser_iov.unwrap()(
                    self.sertype_ptr(),
                    SDK_DATA,
                    1,
                    &iov as *const cyclonedds_sys::iovec,
                    size as usize,
                );
                self.ops().to_ser_unref.unwrap()(serdata_ref, &iov);

                SerData::from_raw(res)
            }
        }

        // ops->from_iox_bufferを呼び出す
        fn serdata_from_iox_buffer(&self, buffer: &IoxChunk<'_, T>) -> Option<Box<SerData<T>>> {
            unsafe {
                let res = self.ops().from_iox_buffer.unwrap()(
                    self.sertype_ptr(),
                    SDK_DATA,
                    std::ptr::null_mut(),
                    buffer.ptr,
                );
                if res.is_null() {
                    None
                } else {
                    SerData::from_raw(res).into()
                }
            }
        }

        // テストで作ったIoxChunkは通常と返却方法が異なるのでマニュアルで解体する
        // NOTE: これでも8つのバッファの確保でSEGVが起きるのでiox_headerのアクセスの確認程度に留める
        fn free_iox_chunk_and_serdata(
            &self,
            mut serdata: Box<SerData<T>>,
            buffer: IoxChunk<'_, T>,
        ) {
            serdata.serdata.iox_chunk = std::ptr::null_mut();
            serdata.serdata.iox_subscriber = std::ptr::null_mut();

            buffer.return_loan();
        }

        // UDP通信向けのシリアライズとバッファ参照コールバックのテスト
        fn serdata_to_ser_ref_for_fragchain(
            &self,
            serdata: &SerData<T>,
            frag_size: u32,
        ) -> Vec<u8> {
            let size = self.get_size(serdata);
            let mut offset = 0;
            let mut result = Vec::new();

            while offset < size {
                let iov = MaybeUninit::<cyclonedds_sys::iovec>::zeroed();
                let chunk_size = frag_size.min(size - offset);
                unsafe {
                    self.ops().to_ser_ref.unwrap()(
                        serdata.as_ptr(),
                        offset as usize,
                        chunk_size as usize,
                        iov.as_ptr() as *mut cyclonedds_sys::iovec,
                    );
                }
                let iov = unsafe { iov.assume_init() };
                let slice =
                    unsafe { std::slice::from_raw_parts(iov.iov_base as *const u8, iov.iov_len) };
                result.extend_from_slice(slice);
                offset += chunk_size;
            }

            result
        }
    }

    // 同一メモリ空間での共有ができないケースではシリアライズデータでの送受信を行う
    #[test_log::test]
    fn test_serdata_ops_iov() -> anyhow::Result<()> {
        let tp: Box<SerType<TestTypeAlloc>> = SerType::<TestTypeAlloc>::new();
        let td = TestTypeAlloc::samples(3);
        let ops = SerDataOps::new(&tp);
        for expect in td {
            let expect = Sample::from(expect);
            let serdata = ops.serdata_from_sample(&expect);
            assert!(serdata.cdr.is_none());
            let recv = ops.serdata_from_ser_iov_seq(&serdata);
            assert!(serdata.cdr.is_some());
            assert!(recv.cdr.is_some());
            if !matches!(recv.sample, SampleData::Uninitialized) {
                panic!("Expected Uninitialized sample data");
            }
            let mut act = Sample::default();
            assert!(ops.to_sample(&recv, &mut act));
            assert_eq!(expect.get_expected().as_ref(), act.try_deref().unwrap());
        }
        Ok(())
    }

    // Shm指定があるケースではiox_chunkを使った送受信を行う
    #[test_log::test]
    #[ignore = "Iceoryx依存"]
    fn test_serdata_ops_iox() -> anyhow::Result<()> {
        crate::common::tests::setup_shm_config();
        let p = DdsParticipant::create(None, None, None)?;
        let pubb = DdsPublisher::create(&p, None, None)?;
        let topic = DdsTopic::<TestTypeAlloc>::create(&p, "serdata_ops_iox", None, None)?;
        let w = Writer::create(&pubb, topic)?;

        let tp = SerType::<TestTypeAlloc>::new();
        let ops = SerDataOps::new(&tp);
        let tpops = SerTypeOps::<TestTypeAlloc>::new(&tp);
        let td = TestTypeAlloc::samples(2);
        for expect in td {
            tracing::trace!("Testing serdata ops with iox chunk {:?}", expect);
            let expect = Sample::from(expect);

            // IoxChunkへの書き込みシーケンス
            let iox_size = tpops.get_serialized_size(&expect);
            // iox_chunkはWriterがプール管理するのでwriterから借りる
            let buffer = w.dds_loan_shared_memory_buffer(iox_size).unwrap();
            assert!(tpops.serialize_into(&expect, &buffer));

            // sertype_opsの関数によってシリアライズされたデータを確認
            let act = cdr::deserialize::<TestTypeAlloc>(buffer.as_slice())?;
            assert_eq!(expect.get_expected().as_ref(), &act);

            // bufferからserdataを作成する
            let recv = ops.serdata_from_iox_buffer(&buffer).unwrap();
            let mut act = Sample::<TestTypeAlloc>::default();
            assert!(ops.to_sample(&recv, &mut act));
            assert_eq!(expect.get_expected().as_ref(), act.try_deref().unwrap());
            ops.free_iox_chunk_and_serdata(recv, buffer);
        }
        Ok(())
    }

    // UDP通信での断片化転送を想定したserdata操作のテスト
    #[test_log::test]
    fn test_serdata_ops_fragchain() -> anyhow::Result<()> {
        let tp: Box<SerType<TestTypeAlloc>> = SerType::<TestTypeAlloc>::new();
        let ops = SerDataOps::new(&tp);
        let td = [
            (64, 1, 32),
            (128, 1, 1024),
            (1024, 1, 2048),
            (1344, 1, 21 * 1024),
        ];

        for (frag_size, id, size) in td {
            let data = TestTypeAlloc::sized_sample(id, size);
            let expect = Sample::from(data);
            let serdata = ops.serdata_from_sample(&expect);
            // 断片化したシリアライズデータを取得する
            let serialized_data = ops.serdata_to_ser_ref_for_fragchain(&serdata, frag_size);
            println!("ser_size {}", serdata.cdr.as_ref().unwrap().len());
            // CDRデータが分割前と同じであることを確認する
            assert_eq!(
                serdata.cdr.as_ref().unwrap().as_slice(),
                serialized_data.as_slice()
            );
        }
        Ok(())
    }
}
