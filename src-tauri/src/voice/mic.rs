//! Живой микрофонный источник для songbird.
//!
//! cpal-колбек пишет моно-f32 в общий буфер (`MicTap`), а songbird читает
//! его через `MicStream` (AsyncRead): пробуждается по waker'у, при закрытии
//! сессии отдаёт EOF и трек завершается. Стерео-дублирование (моно → L=R)
//! делаем здесь же — songbird родной формат 48 кГц stereo f32.
//!
//! Мост в symphonia: `AsyncAdapterStream` (async→sync, блокируется до
//! данных — ровно то, что нужно живому потоку) + `RawAdapter` (магия
//! «SbirdRaw» + sample rate + каналы) → `Input`.
//!
// В headless-сборке (без фичи `audio`) источника нет — приглушаем
// dead_code; в полной сборке предупреждения остаются включёнными.
#![cfg_attr(not(feature = "audio"), allow(dead_code))]

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use songbird::input::{AsyncAdapterStream, AsyncMediaSource, Input, RawAdapter};
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf, SeekFrom};

/// Потолок буфера: 200 мс стерео-f32LE на 48 кГц (48_000·2·4·0.2 ≈ 77 КБ).
const BUF_CAP: usize = 76_800;

/// Пишущая сторона (живёт в cpal-колбеке).
pub struct MicTap {
    buf: Arc<Mutex<VecDeque<u8>>>,
    waker: Arc<Mutex<Option<Waker>>>,
    closed: Arc<AtomicBool>,
}

impl MicTap {
    /// Кадры моно f32 @ 48 кГц → стерео-дубликация → LE-байты в буфер.
    pub fn push_mono(&self, frames: &[f32]) {
        let mut q = self.buf.lock().unwrap();
        for s in frames {
            let b = s.to_le_bytes();
            q.extend(b); // L
            q.extend(b); // R (дубликат)
        }
        if q.len() > BUF_CAP {
            let excess = q.len() - BUF_CAP;
            q.drain(..excess);
        }
        drop(q);
        self.wake();
    }

    /// Конец сессии: читающая сторона получит EOF и трек завершится.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.wake();
    }

    fn wake(&self) {
        if let Some(w) = self.waker.lock().unwrap().take() {
            w.wake();
        }
    }
}

/// Читающая сторона (async-источник для songbird).
pub struct MicStream {
    buf: Arc<Mutex<VecDeque<u8>>>,
    waker: Arc<Mutex<Option<Waker>>>,
    closed: Arc<AtomicBool>,
}

impl AsyncRead for MicStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut q = self.buf.lock().unwrap();
        if q.is_empty() {
            if self.closed.load(Ordering::Relaxed) {
                return Poll::Ready(Ok(())); // EOF: 0 байт записано
            }
            *self.waker.lock().unwrap() = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = buf.remaining().min(q.len());
        let mut chunk = Vec::with_capacity(n);
        for _ in 0..n {
            chunk.push(q.pop_front().unwrap_or(0));
        }
        buf.put_slice(&chunk);
        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for MicStream {
    fn start_seek(self: Pin<&mut Self>, _pos: SeekFrom) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Err(io::ErrorKind::Unsupported.into()))
    }
}

#[async_trait::async_trait]
impl AsyncMediaSource for MicStream {
    fn is_seekable(&self) -> bool {
        false
    }

    async fn byte_len(&self) -> Option<u64> {
        None // живой поток без известной длины
    }
}

/// Пара «колбек-сторона ↔ songbird-источник».
pub fn mic_channel() -> (MicTap, MicStream) {
    let buf = Arc::new(Mutex::new(VecDeque::with_capacity(BUF_CAP)));
    let waker = Arc::new(Mutex::new(None));
    let closed = Arc::new(AtomicBool::new(false));
    (
        MicTap {
            buf: Arc::clone(&buf),
            waker: Arc::clone(&waker),
            closed: Arc::clone(&closed),
        },
        MicStream { buf, waker, closed },
    )
}

/// Собрать songbird-Input из живого микрофонного потока.
pub fn mic_input(stream: MicStream) -> Input {
    let adapter = AsyncAdapterStream::new(Box::new(stream), 64 * 1024);
    Input::from(RawAdapter::new(adapter, 48_000, 2))
}
