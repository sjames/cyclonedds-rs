//! 非同期向けの実装を提供するモジュール

use std::{
    future::{self, Future},
    sync::{Arc, Mutex},
    task::Poll,
};

use cyclonedds_sys::DDSError;
use futures_util::{future::Either, task::AtomicWaker};

use crate::{error::ReaderError, DdsListener, DdsListenerBuilder};

/// cycloneddsのコールバックを使って起こすWakerの型
pub(crate) type AsyncWaker = Arc<(AtomicWaker, Mutex<Option<crate::error::ReaderError>>)>;

/// リーダーとそれに紐づくWakerの有無を保持する型
pub(crate) enum ReaderType {
    /// リーダーに紐付いたWakerがある
    Async(AsyncWaker),
    Sync,
}

pub(crate) fn read<'a, F>(
    reader_type: &'a ReaderType,
    mut readn_from_entity_now: F,
) -> impl Future<Output = Result<usize, ReaderError>> + use<'a, F>
where
    F: FnMut() -> Result<usize, DDSError> + 'a,
{
    if let ReaderType::Async(waker) = reader_type {
        Either::Left(future::poll_fn(move |ctx| {
            // wakerがエラーを持っていたらそれを返す
            if let Some(err) = waker.1.lock().unwrap().take() {
                return Poll::Ready(Err(err));
            }

            match readn_from_entity_now() {
                Ok(len) => Poll::Ready(Ok(len)),
                Err(DDSError::NoData) | Err(DDSError::OutOfResources) => {
                    // データがない場合は次のデータが来るまで待つ
                    waker.0.register(ctx.waker());
                    Poll::Pending
                }
                Err(e) => Poll::Ready(Err(ReaderError::DdsError(e))),
            }
        }))
    } else {
        Either::Right(future::ready(Err(ReaderError::ReaderNotAsync)))
    }
}

/// DataReader用のListenerとWakerの組み合わせを作成する
///
/// データ到達時はOK、Deadline超えとalive count変化時はReaderが判断できるようにエラーをセットする
pub(crate) fn data_reader_listener() -> (DdsListener, ReaderType) {
    let waker = Arc::new((AtomicWaker::new(), Mutex::new(None)));

    let listener = DdsListenerBuilder::new()
        .on_data_available({
            let waker = waker.clone();
            move |_entity| {
                // 新規データが有効になったら起こす
                waker.0.wake();
            }
        })
        .on_requested_deadline_missed({
            let waker = waker.clone();
            move |_entity, _status| {
                // deadlineを守れなかった場合に起こす
                *waker.1.lock().unwrap() = Some(ReaderError::RequestedDeadLineMissed);
                waker.0.wake();
            }
        })
        .on_liveliness_changed({
            let waker = waker.clone();
            move |_entity, status| {
                // publisherのlivelinessが変化したら起こす
                *waker.1.lock().unwrap() = Some(ReaderError::ChangeAliveCount(status.alive_count));
                waker.0.wake();
            }
        })
        .build();
    (listener, ReaderType::Async(waker))
}
