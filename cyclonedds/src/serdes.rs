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

// Rust deserializer for CycloneDDS.
// See discussion at https://github.com/eclipse-cyclonedds/cyclonedds/issues/830

use serde::{de::DeserializeOwned, Serialize};

use std::ptr::NonNull;

use std::time::Duration;
use std::{ops::Deref, sync::Arc};

use cyclonedds_sys::*;
use murmur3::murmur3_32;
use std::io::Cursor;

use crate::serdata::{SampleData, SerData};

// serdata/sertype へ分割した後も、利用者向けの Sample/SampleBuffer API は本モジュールで提供する。

pub trait TopicType: Serialize + DeserializeOwned {
    // generate a non-cryptographic hash of the key values to be used internally
    // in cyclonedds
    fn hash(&self, basehash: u32) -> u32 {
        let cdr = self.key_cdr();
        let mut cursor = Cursor::new(cdr.as_slice());
        murmur3_32(&mut cursor, 0).unwrap() ^ basehash
    }

    fn is_fixed_size() -> bool {
        false
    }

    // The type name for this topic
    //
    // ROS2のメッセージ形式に合わせて/で区切る
    fn typename() -> std::ffi::CString {
        let ty_name_parts: String = std::any::type_name::<Self>()
            .split("::")
            .skip(1)
            .collect::<Vec<_>>()
            .join("/");

        std::ffi::CString::new(ty_name_parts).expect("Unable to create CString for type name")
    }

    /// The default topic_name to use when creating a topic of this type. The default
    /// implementation uses '/' instead of '::' to form a unix like path.
    /// A prefix can optionally be added
    fn topic_name(maybe_prefix: Option<&str>) -> String {
        let topic_name_parts: String = format!(
            "/{}",
            std::any::type_name::<Self>()
                .split("::")
                .skip(1)
                .collect::<Vec<_>>()
                .join("/")
        );

        if let Some(prefix) = maybe_prefix {
            let mut path = String::from(prefix);
            path.push_str(&topic_name_parts);
            path
        } else {
            topic_name_parts
        }
    }

    fn has_key() -> bool;
    // this is the key as defined in the DDS-RTPS spec.
    // KeyHash (PID_KEY_HASH). This function does not
    // hash the key. Use the force_md5_keyhash to know
    // whether to use md5 even if the the key cdr is 16 bytes
    // or shorter.
    fn key_cdr(&self) -> Vec<u8>;

    // force the use of md5 even if the serialized size is less than 16
    // as per the standard, we need to check the potential field size and not the actual.
    fn force_md5_keyhash() -> bool;
}

/// 型がFixed Sizeであることを示すマーカーTrait
pub trait FixedTopicType: TopicType {}

#[derive(Clone)]
pub enum SampleStorage<T> {
    Owned(Arc<T>),
    Loaned(Arc<NonNull<T>>),
}

impl<T> Deref for SampleStorage<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            SampleStorage::Owned(t) => t.deref(),
            SampleStorage::Loaned(t) => unsafe { t.as_ref().as_ref() },
        }
    }
}

/// # Safety
///
/// Sample<T>は内部でRaw Pointerを扱っているが、Send/Syncの実装に問題がないことを保証する
/// ddsi_serdataに有効なポインタが入っている場合は、参照カウントによって生存が保証されているため、複数スレッドからのアクセスが可能
unsafe impl<T> Send for Sample<T> {}

pub struct Sample<T> {
    /// 受信したserdataへのポインタ。参照カウントによって生存しているので、Sampleからアクセスできることが保証されている
    /// 送信利用時には存在しない
    pub(crate) serdata: Option<*mut ddsi_serdata>,
    /// 送信するサンプルデータを保持する。受信利用時には存在しない
    pub(crate) sample: Option<SampleStorage<T>>,
}

impl<T> Sample<T> {
    pub fn try_deref(&self) -> Option<&T> {
        if let Some(serdata) = self.serdata {
            let serdata = SerData::<T>::mut_ref_from_serdata(serdata);
            match &serdata.sample {
                SampleData::Uninitialized => None,
                SampleData::SdkKey => None,
                SampleData::SdkData(it) => Some(it.as_ref()),
                SampleData::ShmData(it) => unsafe { Some(it.as_ref()) },
            }
        } else {
            None
        }
    }

    /// CDR形式のデータを取得する
    pub fn cdr(&self) -> Option<&[u8]> {
        if let Some(serdata) = self.serdata {
            let serdata = SerData::<T>::const_ref_from_serdata(serdata);
            if cfg!(feature = "shm") && !serdata.serdata.iox_chunk.is_null() {
                unsafe {
                    let iox_header = iceoryx_header_from_chunk(serdata.serdata.iox_chunk);
                    let size = (*iox_header).data_size as usize;
                    let buf =
                        std::slice::from_raw_parts(serdata.serdata.iox_chunk as *const u8, size);
                    return Some(buf);
                }
            }
            serdata.cdr.as_deref()
        } else {
            None
        }
    }

    /// write向けに準備されたサンプルを取得
    /// ライブラリ内でのみ利用するので、必ず成功するケースでのみ使うと想定されている
    pub(crate) fn get_expected(&self) -> Arc<T> {
        match &self.sample {
            Some(SampleStorage::Owned(t)) => t.clone(),
            _ => panic!("Sample does not contain owned data"),
        }
    }

    /// データの受信が出来たので対応するserdataを紐付ける
    pub(crate) fn set_serdata(&mut self, serdata: *mut ddsi_serdata) {
        // 紐付けられているデータがあれば開放する
        if let Some(old_serdata) = self.serdata.take() {
            let _ = SerData::<T>::from_raw(old_serdata);
        }
        // Increment the reference count
        unsafe {
            ddsi_serdata_addref(serdata);
        }
        self.serdata = Some(serdata)
    }

    /// サンプルが紐付けられていたら開放する
    pub(crate) fn free_contents(&mut self) {
        if let Some(serdata) = self.serdata.take() {
            unsafe {
                ddsi_serdata_removeref(serdata);
            }
        }
        let _ = self.sample.take();
    }

    /// raw pointerから一時的な参照を取得する
    pub(crate) fn const_ref_from_sample<'a>(sample: *const Sample<T>) -> &'a Self {
        unsafe { &*sample }
    }

    /// raw pointerから可変参照を取得する
    pub(crate) fn mut_ref_from_sample<'a>(sample: *mut Sample<T>) -> &'a mut Self {
        unsafe { &mut *sample }
    }
}

impl<T> Default for Sample<T> {
    fn default() -> Self {
        Self {
            serdata: None,
            sample: None,
        }
    }
}

impl<T> Drop for Sample<T> {
    fn drop(&mut self) {
        if let Some(serdata) = self.serdata {
            unsafe { ddsi_serdata_removeref(serdata) };
        }
    }
}

impl<T> From<Arc<T>> for Sample<T> {
    fn from(data: Arc<T>) -> Self {
        Self {
            serdata: None,
            sample: Some(SampleStorage::Owned(data)),
        }
    }
}

#[derive(Debug)]
pub struct SampleInfoRef<'a> {
    info: &'a cyclonedds_sys::dds_sample_info,
}

impl<'a> SampleInfoRef<'a> {
    /// 受信結果が有効かどうか
    pub fn is_valid(&self) -> bool {
        self.info.valid_data
    }

    /// Sampleが作られた時刻
    ///
    /// 同一マシン内の場合はデータ送信時刻と同じになる
    pub fn source_timestamp(&self) -> Duration {
        Duration::from_nanos(self.info.source_timestamp as u64)
    }
}

/// SampleBufferを保持する構造体。ddsからデータを取り出す際に使う
pub struct SampleBuffer<T> {
    pub(crate) buffer: Vec<Sample<T>>,
    pub(crate) sample_info: Vec<cyclonedds_sys::dds_sample_info>,
    pub(crate) size: usize,
}

impl<'a, T> SampleBuffer<T> {
    pub fn new(len: usize) -> Self {
        Self {
            buffer: (0..len)
                .map(|_| Sample::<T>::default())
                .collect::<Vec<Sample<T>>>(),
            sample_info: vec![cyclonedds_sys::dds_sample_info::default(); len],
            size: 0,
        }
    }

    /// Check if sample is valid. Will panic if out of
    /// bounds.
    pub fn is_valid_sample(&self, index: usize) -> bool {
        self.sample_info[index].valid_data
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// 有効なサンプルを得る(デシリアライズデータ)
    pub fn iter(&'a self) -> impl Iterator<Item = &'a T> {
        self.buffer.iter().filter_map(|sample| sample.try_deref())
    }

    /// 有効なサンプル(デシリアライズデータ)とその送受信情報を得る
    pub fn iter_items(&'a self) -> impl Iterator<Item = (&'a T, SampleInfoRef<'a>)> {
        self.buffer
            .iter()
            .zip(self.sample_info.iter())
            .take(self.size)
            .filter_map(|(sample, info)| {
                sample
                    .try_deref()
                    .map(|data| (data, SampleInfoRef { info }))
            })
    }

    /// 有効なサンプルを得る(CDRアクセス)
    pub fn iter_sample(&'a self) -> impl Iterator<Item = &'a Sample<T>> {
        self.buffer.iter().take(self.size)
    }

    /// 有効なサンプル(CDRアクセス)とその送受信情報を得る
    pub fn iter_sample_items(&'a self) -> impl Iterator<Item = (&'a Sample<T>, SampleInfoRef<'a>)> {
        self.buffer
            .iter()
            .zip(self.sample_info.iter())
            .take(self.size)
            .map(|(sample, info)| (sample, SampleInfoRef { info }))
    }

    /// Get a sample
    pub fn get(&self, index: usize) -> Option<&Sample<T>> {
        self.buffer.get(index)
    }

    /// dds_read/dds_take用にポインタ配列を取得する
    pub(crate) fn as_mut_recv_ptr(&mut self) -> (Vec<&mut Sample<T>>, *mut dds_sample_info) {
        let sample_ptrs: Vec<&mut Sample<T>> = self.buffer.iter_mut().collect();
        (sample_ptrs, self.sample_info.as_mut_ptr())
    }

    pub fn clear(&mut self) {
        for sample in &mut self.buffer {
            sample.free_contents();
        }
    }

    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{sertype::SerType, DdsListener, DdsParticipant, DdsQos, DdsTopic};
    use cyclonedds_derive::Topic;
    use serde::{Deserialize, Serialize};
    use std::ffi::CString;

    // 基本のキーハッシュ。16byte未満ならBEで値を埋める
    // TODO これは正しくない気がする
    #[test]
    fn keyhash_basic() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            y: u32,
        }
        let foo = Foo {
            id: 0x12345678,
            x: 10,
            y: 20,
        };
        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr, vec![0, 0, 0, 0, 0x12u8, 0x34u8, 0x56u8, 0x78u8]);
    }

    // 最大サイズが16byte以上となる場合はmd5を使う
    #[test]
    fn keyhash_simple() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            #[topic_key]
            s: String,
            y: u32,
        }
        let foo = Foo {
            id: 0x12345678,
            x: 10,
            s: String::from("boo"),
            y: 20,
        };
        let key_cdr = foo.key_cdr();
        assert_eq!(
            key_cdr,
            vec![0, 0, 0, 0, 18, 52, 86, 120, 0, 0, 0, 4, 98, 111, 111, 0]
        );
    }

    // 階層構造でもキーハッシュが正しく計算されること
    #[test]
    fn keyhash_nested() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct NestedFoo {
            name: String,
            val: u64,
            #[topic_key]
            instance: u32,
        }

        assert_eq!(
            NestedFoo::typename(),
            CString::new("serdes/test/keyhash_nested/NestedFoo").unwrap()
        );

        impl NestedFoo {
            fn new() -> Self {
                Self {
                    name: "my name".to_owned(),
                    val: 42,
                    instance: 25,
                }
            }
        }

        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            #[topic_key]
            s: String,
            y: u32,
            #[topic_key]
            inner: NestedFoo,
        }
        let foo = Foo {
            id: 0x12345678,
            x: 10,
            s: String::from("boo"),
            y: 20,
            inner: NestedFoo::new(),
        };
        let key_cdr = foo.key_cdr();
        assert_eq!(
            key_cdr,
            vec![0, 0, 0, 0, 18, 52, 86, 120, 0, 0, 0, 4, 98, 111, 111, 0, 0, 0, 0, 25]
        );
    }

    // プリミティブ配列をキーに使うテスト
    #[test]
    fn primitive_array_as_key() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            a: [u8; 8],
            b: u32,
            c: String,
        }

        let foo = Foo {
            a: [0, 0, 0, 0, 0, 0, 0, 0],
            b: 42,
            c: "foo".to_owned(),
        };

        let key_cdr = foo.key_cdr();
        assert_eq!(key_cdr, vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(false, Foo::force_md5_keyhash());
    }

    // 挙動はあっているが、結果が正しくないはず
    // cdrはmd5hash結果が入るべきで、前半が0で埋まっているのは違和感がある
    #[test]
    fn primitive_array_and_string_as_key() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            a: [u8; 8],
            b: u32,
            #[topic_key]
            c: String,
        }

        let foo = Foo {
            a: [0, 0, 0, 0, 0, 0, 0, 0],
            b: 42,
            c: "foo".to_owned(),
        };

        let key_cdr = foo.key_cdr();
        assert_eq!(
            key_cdr,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 102, 111, 111, 0]
        );
        assert_eq!(true, Foo::force_md5_keyhash());
    }

    // topic作成ができるかのテスト
    // sertypeに移動するのが良さそう
    #[test]
    fn basic() {
        #[derive(Serialize, Deserialize, Topic, Default)]
        struct NestedFoo {
            name: String,
            val: u64,
            #[topic_key]
            instance: u32,
        }

        impl NestedFoo {
            fn new() -> Self {
                Self {
                    name: "my name".to_owned(),
                    val: 42,
                    instance: 25,
                }
            }
        }

        #[derive(Serialize, Deserialize, Topic, Default)]
        struct Foo {
            #[topic_key]
            id: i32,
            x: u32,
            #[topic_key]
            s: String,
            y: u32,
            #[topic_key]
            inner: NestedFoo,
        }
        let _foo = Foo {
            id: 0x12345678,
            x: 10,
            s: String::from("boo"),
            y: 20,
            inner: NestedFoo::new(),
        };
        let t = SerType::<Foo>::new();
        let mut t = SerType::into_sertype(t);
        let tt = &mut t as *mut *mut ddsi_sertype;
        unsafe {
            let p = dds_create_participant(0, std::ptr::null_mut(), std::ptr::null_mut());
            let topic_name = CString::new("topic_name").unwrap();
            let topic = dds_create_topic_sertype(
                p,
                topic_name.as_ptr(),
                tt,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );

            dds_delete(topic);
        }
    }
}
