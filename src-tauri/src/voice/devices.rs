//! Аудиоустройства: вывод (динамики) и ввод (микрофон) через cpal
//! (WASAPI на Windows, CoreAudio на macOS, ALSA на Linux).
//!
//! Все колбэки cpal исполняются на потоках аудио-системы — там запрещено
//! блокировать: колбеки только перекладывают сэмплы в/из shared-структур,
//! вся тяжёлая работа (opus, сеть) — в sender-потоке и tokio-тасках.
//!
//! Подбор конфигурации: приоритет 48 кГц (родная частота войса) и F32;
//! при иных возможностях устройства подключается линейный ресемплер
//! (48k ↔ device rate) и при необходимости — миксдаун/дабмикс каналов.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

use crate::voice::jitter::Mixer;
use crate::voice::resample::LinearResampler;
use crate::voice::VoiceControls;

/// Держатель живых потоков устройств.
///
/// `cpal::Stream` на ряде платформ не `Send` (внутри — сырые указатели
/// бэкенда), поэтому их нельзя держать в асинхронной сессии. Guard паркует
/// Stream'ы в выделенном std-потоке, который блокируется на `recv()`:
/// когда Guard дропается (конец сессии), канал закрывается, поток просыпается
/// и дропает Stream'ы — вывод/ввод останавливается детерминированно.
pub struct DeviceGuard {
    _tx: std::sync::mpsc::Sender<()>,
    handle: Option<std::thread::JoinHandle<()>>,
    has_input: bool,
}

impl DeviceGuard {
    /// Открылся ли микрофон (для честного op 5 SPEAKING при старте).
    pub fn has_input(&self) -> bool {
        self.has_input
    }
}

impl Drop for DeviceGuard {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Открывает вывод и ввод. Каждая сторона независима: нет микрофона —
/// сессия работает на приём; нет динамиков — на передачу.
///
/// Открытие происходит ВНУТРИ парковочного потока: `cpal::Stream` на ряде
/// платформ (ALSA) не `Send` — его нельзя ни переносить между потоками,
/// ни держать в асинхронной сессии. Создание, жизнь и дроп — один поток.
pub fn open(
    mixer: &Arc<Mutex<Mixer>>,
    controls: &Arc<Mutex<VoiceControls>>,
    mic: &Arc<Mutex<Vec<i16>>>,
) -> DeviceGuard {
    // Парковка: просыпается при дропе Guard'а (закрытие канала).
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    // has_input → вызывающему (устройства открываются в потоке).
    let (itx, irx) = std::sync::mpsc::channel::<bool>();

    let mixer = Arc::clone(mixer);
    let controls = Arc::clone(controls);
    let mic = Arc::clone(mic);

    let handle = std::thread::spawn(move || {
        let output = open_output(&mixer, &controls);
        let input = open_input(&mic);
        let _ = itx.send(input.is_some());
        // Stream'ы созданы, живут и будут дропнуты в этом же потоке.
        let _hold = (output, input);
        let _ = rx.recv(); // блок до закрытия канала (дроп Guard'а)
    });
    let has_input = irx.recv().unwrap_or(false);

    DeviceGuard {
        _tx: tx,
        handle: Some(handle),
        has_input,
    }
}

/* ------------------------------------------------------------------ */
/* Вывод                                                               */
/* ------------------------------------------------------------------ */

fn pick_output(device: &cpal::Device) -> Option<(StreamConfig, SampleFormat)> {
    let ranges: Vec<_> = device.supported_output_configs().ok()?.collect();
    for fmt in [SampleFormat::F32, SampleFormat::I16] {
        for ch in [2u16, 1u16] {
            if let Some(r) = ranges.iter().find(|r| {
                r.sample_format() == fmt
                    && r.channels() == ch
                    && r.min_sample_rate().0 <= 48_000
                    && r.max_sample_rate().0 >= 48_000
            }) {
                let cfg = r.with_sample_rate(cpal::SampleRate(48_000));
                let fmt = cfg.sample_format();
                return Some((cfg.into(), fmt));
            }
        }
    }
    let cfg = device.default_output_config().ok()?;
    let fmt = cfg.sample_format();
    if !matches!(fmt, SampleFormat::F32 | SampleFormat::I16) {
        return None;
    }
    Some((cfg.into(), fmt))
}

/// Состояние колбэка вывода: микшер → (ресемпл) → каналы устройства.
struct OutTap {
    mixer: Arc<Mutex<Mixer>>,
    controls: Arc<Mutex<VoiceControls>>,
    dev_rate: u32,
    dev_ch: usize,
    /// 48k-стерео → device-rate-стерео (None при 48k).
    res: Option<LinearResampler>,
    /// Готовые device-rate-стерео сэмплы, ожидающие колбэк.
    ready: Vec<f32>,
    /// Скретч под один чанк 48k-стерео для ресемплера.
    scratch: Vec<f32>,
}

impl OutTap {
    fn fill(&mut self, out: &mut [f32]) {
        let deaf = self.controls.lock().unwrap().self_deaf;
        let need_frames = out.len() / self.dev_ch;

        // Быстрый путь: устройство и так 48k стерео — микшируем прямо в колбэк.
        if self.dev_rate == 48_000 && self.dev_ch == 2 {
            self.mixer.lock().unwrap().mix_into(out, deaf);
            return;
        }

        // Общий путь: добираем device-rate-стерео из 48k через ресемплер.
        while self.ready.len() < need_frames * 2 {
            self.scratch.resize(960, 0.0); // 10 мс стерео на шаг
            self.mixer.lock().unwrap().mix_into(&mut self.scratch, deaf);
            if let Some(r) = self.res.as_mut() {
                self.ready.extend(r.push(&self.scratch));
            } else {
                self.ready.extend_from_slice(&self.scratch); // не бывает, но безопасно
            }
            if self.ready.len() >= need_frames * 2 {
                break;
            }
        }
        // Раскладка стерео → каналы устройства.
        let mut idx = 0usize;
        for frame in out.chunks_mut(self.dev_ch) {
            let l = self.ready.get(idx).copied().unwrap_or(0.0);
            let rch = self.ready.get(idx + 1).copied().unwrap_or(0.0);
            match self.dev_ch {
                1 => frame[0] = (l + rch) * 0.5,
                2 => {
                    frame[0] = l;
                    frame[1] = rch;
                }
                n => {
                    // >2: первый двухканальный след, остальное — тишина.
                    frame[0] = l;
                    frame[1] = rch;
                    for c in frame.iter_mut().skip(2).take(n) {
                        *c = 0.0;
                    }
                }
            }
            idx += 2;
        }
        if self.ready.len() > need_frames * 2 {
            self.ready.drain(..need_frames * 2);
        } else {
            self.ready.clear();
        }
    }
}

fn open_output(mixer: &Arc<Mutex<Mixer>>, controls: &Arc<Mutex<VoiceControls>>) -> Option<Stream> {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            tracing::warn!("voice: устройство вывода не найдено — приём без звука");
            return None;
        }
    };
    let name = device.name().unwrap_or_default();
    let (config, fmt) = match pick_output(&device) {
        Some(v) => v,
        None => {
            tracing::warn!("voice: у «{name}» нет поддерживаемой конфигурации (F32/I16)");
            return None;
        }
    };
    let dev_rate = config.sample_rate.0;
    let dev_ch = config.channels.max(1) as usize;
    let res = (dev_rate != 48_000).then(|| LinearResampler::new(48_000, dev_rate, 2));
    tracing::info!(
        "voice: вывод через «{name}»: {} кГц, {} кан., {:?}",
        dev_rate / 1000,
        dev_ch,
        fmt
    );

    let mut tap = OutTap {
        mixer: Arc::clone(mixer),
        controls: Arc::clone(controls),
        dev_rate,
        dev_ch,
        res,
        ready: Vec::new(),
        scratch: Vec::new(),
    };
    let err_fn = |e| tracing::warn!("voice: ошибка вывода: {e}");

    let stream = match fmt {
        SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| tap.fill(data),
                err_fn,
                None,
            )
            .ok(),
        SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let mut tmp = vec![0f32; data.len()];
                    tap.fill(&mut tmp);
                    for (o, s) in data.iter_mut().zip(tmp) {
                        *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                    }
                },
                err_fn,
                None,
            )
            .ok(),
        _ => None,
    };
    match stream {
        Some(s) => match s.play() {
            Ok(()) => Some(s),
            Err(e) => {
                tracing::warn!("voice: не удалось запустить вывод: {e}");
                None
            }
        },
        None => {
            tracing::warn!("voice: build_output_stream отказал");
            None
        }
    }
}

/* ------------------------------------------------------------------ */
/* Ввод                                                                */
/* ------------------------------------------------------------------ */

fn pick_input(device: &cpal::Device) -> Option<(StreamConfig, SampleFormat)> {
    let ranges: Vec<_> = device.supported_input_configs().ok()?.collect();
    for fmt in [SampleFormat::F32, SampleFormat::I16] {
        if let Some(r) = ranges.iter().find(|r| {
            r.sample_format() == fmt
                && r.min_sample_rate().0 <= 48_000
                && r.max_sample_rate().0 >= 48_000
        }) {
            let cfg = r.with_sample_rate(cpal::SampleRate(48_000));
            let fmt = cfg.sample_format();
            return Some((cfg.into(), fmt));
        }
    }
    let cfg = device.default_input_config().ok()?;
    let fmt = cfg.sample_format();
    if !matches!(fmt, SampleFormat::F32 | SampleFormat::I16) {
        return None;
    }
    Some((cfg.into(), fmt))
}

/// Состояние колбэка ввода: устройство → моно 48k → аккумулятор для кодера.
struct InTap {
    mic: Arc<Mutex<Vec<i16>>>,
    dev_ch: usize,
    /// device-rate-моно → 48k-моно (None при 48k).
    res: Option<LinearResampler>,
}

impl InTap {
    fn push(&mut self, frames: &[f32]) {
        // frames: интерлив device-каналов. Сводим в моно.
        let n = frames.len() / self.dev_ch.max(1);
        let mut mono = Vec::with_capacity(n);
        for i in 0..n {
            let mut acc = 0f32;
            for c in 0..self.dev_ch {
                acc += frames[i * self.dev_ch + c];
            }
            mono.push(acc / self.dev_ch.max(1) as f32);
        }
        let at48k: Vec<f32> = match self.res.as_mut() {
            Some(r) => r.push(&mono),
            None => mono,
        };
        if at48k.is_empty() {
            return;
        }
        let mut mic = self.mic.lock().unwrap();
        for s in at48k {
            mic.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
        // Страховка от разбухания, если кодер застрял: держим ≤ 80 мс.
        const CAP: usize = 3840;
        if mic.len() > CAP {
            let excess = mic.len() - CAP;
            mic.drain(..excess);
        }
    }
}

fn open_input(mic: &Arc<Mutex<Vec<i16>>>) -> Option<Stream> {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            tracing::warn!("voice: микрофон не найден — передача недоступна");
            return None;
        }
    };
    let name = device.name().unwrap_or_default();
    let (config, fmt) = match pick_input(&device) {
        Some(v) => v,
        None => {
            tracing::warn!("voice: у микрофона «{name}» нет поддерживаемой конфигурации");
            return None;
        }
    };
    let dev_rate = config.sample_rate.0;
    let dev_ch = config.channels.max(1) as usize;
    let res = (dev_rate != 48_000).then(|| LinearResampler::new(dev_rate, 48_000, 1));
    tracing::info!(
        "voice: ввод через «{name}»: {} кГц, {} кан., {:?}",
        dev_rate / 1000,
        dev_ch,
        fmt
    );

    let mut tap = InTap {
        mic: Arc::clone(mic),
        dev_ch,
        res,
    };
    let err_fn = |e| tracing::warn!("voice: ошибка ввода: {e}");

    let stream = match fmt {
        SampleFormat::F32 => device
            .build_input_stream(&config, move |data: &[f32], _| tap.push(data), err_fn, None)
            .ok(),
        SampleFormat::I16 => device
            .build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let tmp: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    tap.push(&tmp);
                },
                err_fn,
                None,
            )
            .ok(),
        _ => None,
    };
    match stream {
        Some(s) => match s.play() {
            Ok(()) => Some(s),
            Err(e) => {
                tracing::warn!("voice: не удалось запустить ввод: {e}");
                None
            }
        },
        None => {
            tracing::warn!("voice: build_input_stream отказал");
            None
        }
    }
}
