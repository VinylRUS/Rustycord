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
//! Телеметрия (`MicStats`): сколько байт записал cpal-колбек и сколько
//! реально прочитал songbird + пик уровня. Пишется из аудио-потоков
//! (atomics), читается монитор-таской сессии (coordinator.rs): лог каждые
//! 10 с и событие `mic_level` в UI каждые 500 мс. Без неё «меня не слышно»
//! не диагностировать: тишь на захвате / мёртвый мост / застрявший трек
//! выглядят снаружи одинаково.
// В headless-сборке (без фичи `audio`) источника нет — приглушаем
// dead_code; в полной сборке предупреждения остаются включёнными.
#![cfg_attr(not(feature = "audio"), allow(dead_code))]

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use songbird::input::{AsyncAdapterStream, AsyncMediaSource, Input, RawAdapter};
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf, SeekFrom};

/// Потолок буфера: 200 мс стерео-f32LE на 48 кГц (48_000·2·4·0.2 ≈ 77 КБ).
const BUF_CAP: usize = 76_800;

/// Телеметрия микрофонного тракта (все поля — atomics: пишутся из
/// cpal-колбека и моста, читаются монитор-таской).
#[derive(Default)]
pub struct MicStats {
    /// Байт записано cpal-колбеком (после стерео-дублирования: кадр = 8 Б).
    pub bytes_in: AtomicU64,
    /// Байт прочитано songbird'ом через мост (AsyncAdapterStream).
    pub bytes_out: AtomicU64,
    /// f32-биты максимума |сэмпла| с последнего `take_peak` (у
    /// неотрицательных f32 битовый порядок совпадает с порядком значений,
    /// поэтому fetch_max валиден).
    peak: AtomicU32,
}

impl MicStats {
    /// Пик уровня с последнего вызова; сбрасывает счётчик в ноль.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }

    /// Зафиксировать максимум |сэмпла| в кадре (вызов из cpal-колбека).
    fn note_peak(&self, frames: &[f32]) {
        let mut m = 0.0f32;
        for s in frames {
            let a = s.abs();
            if a > m {
                m = a;
            }
        }
        // NaN не случается у живого захвата, но биты NaN «прилипнут»
        // к fetch_max навсегда — пропускаем.
        if m.is_finite() {
            self.peak.fetch_max(m.to_bits(), Ordering::Relaxed);
        }
    }
}

/// Пишущая сторона (живёт в cpal-колбеке).
pub struct MicTap {
    buf: Arc<Mutex<VecDeque<u8>>>,
    waker: Arc<Mutex<Option<Waker>>>,
    closed: Arc<AtomicBool>,
    stats: Arc<MicStats>,
}

impl MicTap {
    /// Кадры моно f32 @ 48 кГц → стерео-дубликация → LE-байты в буфер.
    pub fn push_mono(&self, frames: &[f32]) {
        // Телеметрия: пик до дублирования (уровень канала), байты — после.
        self.stats.note_peak(frames);
        self.stats
            .bytes_in
            .fetch_add(frames.len() as u64 * 8, Ordering::Relaxed);
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
    stats: Arc<MicStats>,
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
        self.stats.bytes_out.fetch_add(n as u64, Ordering::Relaxed);
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

/// Пара «колбек-сторона ↔ songbird-источник» + телеметрия тракта.
pub fn mic_channel() -> (MicTap, MicStream, Arc<MicStats>) {
    let buf = Arc::new(Mutex::new(VecDeque::with_capacity(BUF_CAP)));
    let waker = Arc::new(Mutex::new(None));
    let closed = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(MicStats::default());
    (
        MicTap {
            buf: Arc::clone(&buf),
            waker: Arc::clone(&waker),
            closed: Arc::clone(&closed),
            stats: Arc::clone(&stats),
        },
        MicStream {
            buf,
            waker,
            closed,
            stats: Arc::clone(&stats),
        },
        stats,
    )
}

/// Собрать songbird-Input из живого микрофонного потока.
pub fn mic_input(stream: MicStream) -> Input {
    let adapter = AsyncAdapterStream::new(Box::new(stream), 64 * 1024);
    Input::from(RawAdapter::new(adapter, 48_000, 2))
}
