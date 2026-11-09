//! Обработка микрофонного сигнала перед кодированием: шумоподавление
//! (STFT + винеровская оценка усиления) и автоматическая регулировка
//! усиления (AGC) с пиковым лимитером.
//!
//! Встраивается в InTap (devices.rs) между ресемплером и MicTap — на вход
//! и выход поступает моно f32 @ 48 кГц произвольными чанками.
//!
//! Шумоподавление (как у Krisp/RNNoise, но проще):
//! - STFT: окно 1024 (≈21 мс), прыжок 512 (≈10.7 мс), окно Ханна,
//!   обратное сложение с перекрытием (WOLA) и табличной нормализацией;
//! - оценка шума: сглаживание мощности бинов + min-трекинг со счётчиком
//!   устаревания; во время речи оценка заморожена (не догоняет речь);
//! - винеровский гейн g = ξ/(1+ξ) с over-subtraction и полом по силе,
//!   временной hangover (анти-«музыкальный шум») и частотное сглаживание;
//! - временной гейт с гистерезисом VAD и рампой — добивает остаток шума
//!   в паузах.
//!
//! AGC: RMS речи ведётся экспо-энвелопой, усиление стремится к цели
//! −20 dBFS с ограничением скорости (быстро вверх, медленно вниз) и
//! замораживается в тишине (не разгоняет шум). Лимитер — пер-сэмпловый
//! (мгновенная атака, медленный релиз), защищает от клипа всегда, даже
//! когда обе обработки выключены.
//!
//! Всё считается прямо в аудио-колбеке: один FFT-1024 на 10.7 мс —
//! единицы микросекунд, аллокаций на стационарном пути нет.

use serde::{Deserialize, Serialize};

/// Частота потока (обработчик стоит после ресемплера).
pub const SAMPLE_RATE: u32 = 48_000;
/// Размер окна STFT (степень двойки для FFT).
pub const WIN: usize = 1024;
/// Прыжок между кадрами (50 % перекрытие).
pub const HOP: usize = WIN / 2;
/// Количество полезных бинов (0..=N/2).
pub const BINS: usize = WIN / 2 + 1;

/* ------------------------------------------------------------------ */
/* Конфигурация и метрики                                              */
/* ------------------------------------------------------------------ */

/// Сила шумодава: насколько агрессивно давится спектр и паузы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NsStrength {
    Low,
    Medium,
    High,
}

impl NsStrength {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// Over-subtraction: насколько сильнее шума вычитаем из спектра.
    fn oversub(self) -> f32 {
        match self {
            Self::Low => 1.0,
            Self::Medium => 1.55,
            Self::High => 2.1,
        }
    }

    /// Пол спектрального гейна (ниже не опускаемся).
    fn floor(self) -> f32 {
        match self {
            Self::Low => 0.18,
            Self::Medium => 0.09,
            Self::High => 0.045,
        }
    }

    /// Уровень гейта в паузе (добивание остаточного шума).
    fn gate_floor(self) -> f32 {
        match self {
            Self::Low => 0.35,
            Self::Medium => 0.18,
            Self::High => 0.07,
        }
    }
}

/// Настройки обработки (пишутся командой set_voice_dsp, читаются
/// аудио-колбеком через Arc<Mutex<…>>).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DspConfig {
    pub ns_enabled: bool,
    pub ns_strength: NsStrength,
    pub agc_enabled: bool,
}

impl Default for DspConfig {
    fn default() -> Self {
        Self {
            ns_enabled: true,
            ns_strength: NsStrength::Medium,
            agc_enabled: true,
        }
    }
}

impl DspConfig {
    /// Разбор значения ns_strength, приходящего строкой из фронтенда.
    pub fn strength_from_str(s: &str) -> Option<NsStrength> {
        NsStrength::from_str(s)
    }
}

/// Живая телеметрия обработки — рисуется в настройках «Голос».
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DspMetrics {
    /// RMS входа (до обработки), dBFS.
    pub input_db: f32,
    /// RMS выхода (уходит в opus), dBFS.
    pub output_db: f32,
    /// Оценка шума (RMS эквивалент), dBFS.
    pub noise_db: f32,
    /// Текущее усиление AGC, dB.
    pub agc_db: f32,
    /// Сколько снял шумодав на последнем кадре, dB (0 = не давит).
    pub ns_db: f32,
    /// VAD: сейчас речь.
    pub speech: bool,
}

impl Default for DspMetrics {
    fn default() -> Self {
        Self {
            input_db: -100.0,
            output_db: -100.0,
            noise_db: -100.0,
            agc_db: 0.0,
            ns_db: 0.0,
            speech: false,
        }
    }
}

/// Снимок состояния для UI: метрики + жива ли вообще голосовая сессия.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DspStatus {
    /// Открыт ли сейчас микрофон (есть активная сессия).
    pub active: bool,
    pub metrics: DspMetrics,
}

/* ------------------------------------------------------------------ */
/* FFT (радикс-2, итеративный, таблицы считаются один раз)             */
/* ------------------------------------------------------------------ */

struct Fft {
    n: usize,
    rev: Vec<u32>,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
}

impl Fft {
    /// `n` — степень двойки.
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two(), "FFT: n должна быть степенью двойки");
        let log_n = n.trailing_zeros();
        let mut rev = vec![0u32; n];
        for (i, r) in rev.iter_mut().enumerate() {
            // разворот бит i → rev[i]
            *r = (i as u32).reverse_bits() >> (32 - log_n);
        }
        // таблица корней e^{-2πik/n}, k < n/2
        let half = n / 2;
        let mut tw_re = vec![0f32; half];
        let mut tw_im = vec![0f32; half];
        for k in 0..half {
            let a = -2.0 * std::f32::consts::PI * (k as f32) / (n as f32);
            tw_re[k] = a.cos();
            tw_im[k] = a.sin();
        }
        Self {
            n,
            rev,
            tw_re,
            tw_im,
        }
    }

    /// Прямое ДПФ, in-place. Знак — «как в учебнике» (e^{-iω}).
    fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        debug_assert_eq!(re.len(), self.n);
        debug_assert_eq!(im.len(), self.n);
        for i in 0..self.n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2usize;
        while len <= self.n {
            let half = len >> 1;
            let stride = self.n / len; // шаг по таблице твистлов
            let mut base = 0;
            while base < self.n {
                for k in 0..half {
                    let wr = self.tw_re[k * stride];
                    let wi = self.tw_im[k * stride];
                    let i0 = base + k;
                    let i1 = i0 + half;
                    let xr = re[i1] * wr - im[i1] * wi;
                    let xi = re[i1] * wi + im[i1] * wr;
                    re[i1] = re[i0] - xr;
                    im[i1] = im[i0] - xi;
                    re[i0] += xr;
                    im[i0] += xi;
                }
                base += len;
            }
            len <<= 1;
        }
    }

    /// Обратное ДПФ, in-place, результат умножен на 1/n.
    /// IDFT(x) = conj(FFT(conj(x)))/n.
    fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        for v in im.iter_mut() {
            *v = -*v;
        }
        self.forward(re, im);
        let n = self.n as f32;
        for r in re.iter_mut() {
            *r /= n;
        }
        for v in im.iter_mut() {
            *v = -(*v / n);
        }
    }
}

/* ------------------------------------------------------------------ */
/* Процессор                                                           */
/* ------------------------------------------------------------------ */

/// Целевой RMS речи после AGC (−20 dBFS — «дискордовский» уровень).
const AGC_TARGET_RMS: f32 = 0.1;
/// Пределы усиления AGC, dB.
const AGC_MAX_DB: f32 = 30.0;
const AGC_MIN_DB: f32 = -12.0;
/// Скорость подстройки усиления, dB/с: вверх быстро (компенсация в
/// пределах фразы), вниз мягче (не «дышит» на пиках).
const AGC_UP_DBPS: f32 = 20.0;
const AGC_DOWN_DBPS: f32 = 8.0;
/// Затухание огибающей RMS речи на кадр (полупериод ~1.4 с).
const AGC_ENV_DECAY: f32 = 0.995;

/// Порог VAD (гистерезис): открываемся выше SNR_ON, закрываемся ниже SNR_OFF.
const VAD_SNR_ON_DB: f32 = 7.0;
const VAD_SNR_OFF_DB: f32 = 4.0;
/// Первые кадры — калибровка оценки шума, VAD закрыт (~32 мс).
const VAD_CALIBRATION: u32 = 3;
/// Hangover гейта: сколько кадров тишины держим его открытым (~107 мс).
const GATE_HANGOVER: u32 = 10;

/// Порог лимитера и финальный кламп.
const LIMITER_THR: f32 = 0.85;
const OUT_CLAMP: f32 = 0.99;

/// Сглаживание мощности бинов: 0.65 на кадр (~30 мс).
const POW_SMOOTH: f32 = 0.65;
/// Трекинг шума в тишине: симметричное сглаживание (~32 мс).
/// Асимметрия (быстро вниз / медленно вверх) здесь НЕ ГОДИТСЯ:
/// у мощности бинов экспоненциальное распределение, и односторонний
/// трекер оседает на 5–7 dB ниже среднего — SNR раздувается и VAD
/// ложно открывается на ровном шуме. Защиту оценки от речи даёт не
/// асимметрия, а заморозка при открытом VAD.
const NOISE_TRACK: f32 = 0.7;
/// Hangover спектрального гейна (анти-«музыкальный шум»).
const GAIN_HANGOVER: f32 = 0.55;

pub struct VoiceDsp {
    cfg: DspConfig,
    last_ns_on: bool,

    // FFT + STFT
    fft: Fft,
    win: Vec<f32>,
    /// WOLA-нормализация: norm[i] = w²[i] + w²[i+HOP], период HOP.
    norm: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Буфер входа: хвост WIN-HOP + новые сэмплы (прайминг нулями).
    buf: Vec<f32>,
    /// Накопитель обратного сложения.
    ola: Vec<f32>,

    // Спектральное состояние
    smooth_pow: Vec<f32>,
    noise_pow: Vec<f32>,
    prev_gain: Vec<f32>,
    /// Гейн после частотного сглаживания (скретч).
    gain: Vec<f32>,

    // VAD / гейт
    speech: bool,
    silence_frames: u32,
    gate: f32,
    /// Счётчик обработанных кадров (первые кадры — калибровка шума,
    /// VAD принудительно закрыт: на нулевой истории SNR не определён).
    frames: u32,
    /// Первый кадр ещё не обработан (инициализация оценки шума).
    first_frame: bool,

    // AGC / лимитер
    speech_env: f32,
    agc_gain: f32,
    lim_env: f32,
    lim_gain: f32,
    /// Прошлое усиление AGC для линейной интерполяции внутри кадра.
    agc_prev: f32,

    // DC-блокер (двухполюсный one-pole highpass, ~19 Гц)
    dc_x: f32,
    dc_y: f32,

    // Метрики
    metrics: DspMetrics,
    /// Парсевал: N·Σw² — перевод спектральной мощности в RMS времени.
    parseval: f32,
}

impl VoiceDsp {
    pub fn new(cfg: DspConfig) -> Self {
        // Периодическое окно Ханна: sin²(πn/N).
        let win: Vec<f32> = (0..WIN)
            .map(|i| {
                let s = (std::f32::consts::PI * i as f32 / WIN as f32).sin();
                s * s
            })
            .collect();
        // Нормализация WOLA: два перекрывающихся окна Ханна.
        let norm: Vec<f32> = (0..HOP)
            .map(|i| win[i] * win[i] + win[i + HOP] * win[i + HOP])
            .collect();
        let nmin = norm.iter().cloned().fold(f32::MAX, f32::min);
        debug_assert!(
            nmin > 0.4,
            "WOLA-нормализация не должна проваливаться: {nmin}"
        );
        let win_sq_sum: f32 = win.iter().map(|w| w * w).sum();
        let parseval = WIN as f32 * win_sq_sum;
        Self {
            cfg,
            last_ns_on: cfg.ns_enabled,
            fft: Fft::new(WIN),
            win,
            norm,
            re: vec![0.0; WIN],
            im: vec![0.0; WIN],
            buf: vec![0.0; WIN - HOP], // прайминг: алгоритмическая задержка 512 сэмплов
            ola: vec![0.0; WIN],
            smooth_pow: vec![0.0; BINS],
            noise_pow: vec![0.0; BINS],
            prev_gain: vec![1.0; BINS],
            gain: vec![1.0; BINS],
            speech: false,
            silence_frames: GATE_HANGOVER,
            gate: 1.0,
            frames: 0,
            first_frame: true,
            speech_env: 1.0e-6,
            agc_gain: 1.0,
            lim_env: 0.0,
            lim_gain: 1.0,
            agc_prev: 1.0,
            dc_x: 0.0,
            dc_y: 0.0,
            metrics: DspMetrics::default(),
            parseval,
        }
    }

    pub fn config(&self) -> DspConfig {
        self.cfg
    }

    /// Обновить настройки на лету (следующий кадр обработается по-новому).
    pub fn set_config(&mut self, cfg: DspConfig) {
        if cfg.ns_enabled && !self.last_ns_on {
            // после паузы обработки спектральный гейн разгоняем с единицы
            self.prev_gain.iter_mut().for_each(|g| *g = 1.0);
            self.gate = 1.0;
            self.silence_frames = 0;
        }
        self.last_ns_on = cfg.ns_enabled;
        self.cfg = cfg;
    }

    pub fn metrics(&self) -> DspMetrics {
        self.metrics
    }

    /// Обработать очередной чанк моно-сэмплов; результат дописывается в `out`.
    ///
    /// При выключенных обеих обработках STFT пропускается (транзит),
    /// остаются DC-блокер и лимитер.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        // метрики входа: RMS до всего
        let mut acc = 0f64;
        for s in input {
            acc += f64::from(*s) * f64::from(*s);
        }
        let in_rms = (acc / input.len() as f64).sqrt() as f32;
        let in_db = 20.0 * (in_rms + 1.0e-8).log10();

        let base = out.len();
        if self.cfg.ns_enabled || self.cfg.agc_enabled {
            // STFT-путь
            for s in input {
                let x = self.dc_block(*s);
                self.buf.push(x);
            }
            while self.buf.len() >= WIN {
                self.step(out);
            }
        } else {
            // транзит: DC-блокер + лимитер (усиление не трогаем — всё выключено)
            for s in input {
                let x = self.dc_block(*s);
                out.push(self.limit(x));
            }
        }

        // метрики выхода
        if out.len() > base {
            let mut acc = 0f64;
            for s in &out[base..] {
                acc += f64::from(*s) * f64::from(*s);
            }
            let out_rms = (acc / (out.len() - base) as f64).sqrt() as f32;
            let out_db = 20.0 * (out_rms + 1.0e-8).log10();
            self.metrics.output_db += 0.3 * (out_db - self.metrics.output_db);
        }
        self.metrics.input_db += 0.3 * (in_db - self.metrics.input_db);
        self.metrics.agc_db = 20.0 * self.agc_gain.log10();
        self.metrics.speech = self.speech;
    }

    /// Один кадр STFT: анализ → гейн → синтез → гейт/AGC/лимитер кадра.
    fn step(&mut self, out: &mut Vec<f32>) {
        // 1. Окно и прямое FFT
        for i in 0..WIN {
            self.re[i] = self.buf[i] * self.win[i];
            self.im[i] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);

        // 2. Анализ: сглаженная мощность, VAD, трекинг шума
        let mut sum_smooth = 0f32;
        let mut sum_noise = 0f32;
        for k in 0..BINS {
            let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
            self.smooth_pow[k] = POW_SMOOTH * self.smooth_pow[k] + (1.0 - POW_SMOOTH) * p;
            sum_smooth += self.smooth_pow[k];
            sum_noise += self.noise_pow[k];
        }
        // первый кадр: шум = текущий спектр — ДО решения VAD, иначе
        // SNR делится на ноль, «речь» защёлкивается и замораживает
        // заниженную оценку навсегда (сессия стартует в тишине/шуме)
        if self.first_frame {
            self.noise_pow.copy_from_slice(&self.smooth_pow);
            self.first_frame = false;
        }
        let snr_db = 10.0 * ((sum_smooth + 1.0e-12) / (sum_noise + 1.0e-12)).log10();
        self.frames += 1;
        // VAD: гистерезис + принудительно закрыт на калибровочных кадрах
        self.speech = if self.frames <= VAD_CALIBRATION {
            false
        } else if self.speech {
            snr_db > VAD_SNR_OFF_DB
        } else {
            snr_db > VAD_SNR_ON_DB
        };
        if self.speech {
            self.silence_frames = 0;
        } else {
            self.silence_frames = self.silence_frames.saturating_add(1);
        }
        // трекинг шума: в тишине оценка следует за сглаженным спектром
        // (симметрично — без смещения равновесия), во время речи —
        // заморожена, чтобы не догонять речь.
        if !self.speech {
            for k in 0..BINS {
                self.noise_pow[k] =
                    NOISE_TRACK * self.noise_pow[k] + (1.0 - NOISE_TRACK) * self.smooth_pow[k];
            }
        }

        // 3. Спектральный гейн
        let mut mean_gain = 0f32;
        if self.cfg.ns_enabled {
            let oversub = self.cfg.ns_strength.oversub();
            let floor = self.cfg.ns_strength.floor();
            for k in 0..BINS {
                let noise = (self.noise_pow[k] + 1.0e-12).max(oversub * self.noise_pow[k]);
                // априорное SNR (вычитаем шум из смеси)
                let xi = ((self.smooth_pow[k] - noise) / (self.noise_pow[k] + 1.0e-12)).max(0.0);
                let mut g = xi / (1.0 + xi);
                if g < floor {
                    g = floor;
                }
                if g < GAIN_HANGOVER * self.prev_gain[k] {
                    g = GAIN_HANGOVER * self.prev_gain[k];
                }
                self.prev_gain[k] = g;
            }
            // частотное сглаживание [0.25, 0.5, 0.25]
            let n = BINS;
            self.gain[0] = self.prev_gain[0];
            self.gain[n - 1] = self.prev_gain[n - 1];
            for k in 1..n - 1 {
                self.gain[k] = 0.25 * self.prev_gain[k - 1]
                    + 0.5 * self.prev_gain[k]
                    + 0.25 * self.prev_gain[k + 1];
            }
            for k in 0..BINS {
                let g = self.gain[k];
                self.re[k] *= g;
                self.im[k] *= g;
                // зеркальные бины — для честного вещественного сигнала
                if k > 0 && k < WIN - k {
                    self.re[WIN - k] *= g;
                    self.im[WIN - k] *= g;
                }
                mean_gain += g;
            }
            mean_gain /= BINS as f32;
        } else {
            mean_gain = 1.0;
        }

        // 4. Обратное FFT, синтезное окно, WOLA
        self.fft.inverse(&mut self.re, &mut self.im);
        for j in 0..WIN {
            self.ola[j] += self.re[j] * self.win[j];
        }

        // 5. Выдача HOP сэмплов с нормализацией
        let hop_out_base = out.len();
        for k in 0..HOP {
            out.push(self.ola[k] / self.norm[k]);
        }
        self.ola.drain(..HOP);
        self.ola.resize(WIN, 0.0);
        self.buf.drain(..HOP);

        // 6. Гейт, AGC, лимитер на выданном кадре
        self.post_process(out, hop_out_base);

        // 7. Метрики кадра (Парсевал: спектр → RMS времени;
        // ×2 — учитываем зеркальные бины)
        let noise_rms = (2.0 * self.noise_pow.iter().sum::<f32>() / self.parseval).sqrt();
        self.metrics.noise_db = 20.0 * (noise_rms + 1.0e-8).log10();
        self.metrics.ns_db = 20.0 * (mean_gain + 1.0e-8).log10();
    }

    /// Временная обработка выданного кадра: гейт → AGC → лимитер.
    fn post_process(&mut self, out: &mut [f32], base: usize) {
        // RMS кадра (после шумодава, до AGC)
        let mut acc = 0f32;
        for s in &out[base..] {
            acc += s * s;
        }
        let rms = (acc / HOP as f32 + 1.0e-12).sqrt();

        // --- гейт ---
        if self.cfg.ns_enabled {
            let target = if self.silence_frames >= GATE_HANGOVER {
                self.cfg.ns_strength.gate_floor()
            } else {
                1.0
            };
            let k = if target > self.gate { 0.5 } else { 0.12 };
            self.gate += k * (target - self.gate);
        } else {
            self.gate = 1.0;
        }

        // --- AGC ---
        self.agc_prev = self.agc_gain;
        if self.cfg.agc_enabled {
            if self.speech {
                // огибающая RMS речи: мгновенно вверх, медленно вниз
                self.speech_env = if rms > self.speech_env {
                    rms
                } else {
                    AGC_ENV_DECAY * self.speech_env
                };
                let target = (AGC_TARGET_RMS / self.speech_env)
                    .clamp(10f32.powf(AGC_MIN_DB / 20.0), 10f32.powf(AGC_MAX_DB / 20.0));
                // к цели — с ограничением скорости (в децибелах)
                let cur_db = 20.0 * self.agc_gain.log10();
                let tgt_db = 20.0 * target.log10();
                let max_step = if tgt_db > cur_db {
                    AGC_UP_DBPS * HOP as f32 / SAMPLE_RATE as f32
                } else {
                    AGC_DOWN_DBPS * HOP as f32 / SAMPLE_RATE as f32
                };
                let new_db = if (tgt_db - cur_db).abs() < max_step {
                    tgt_db
                } else {
                    cur_db + max_step * (tgt_db - cur_db).signum()
                };
                self.agc_gain = 10f32.powf(new_db / 20.0);
            }
            // в тишине усиление заморожено — шум не разгоняем
        } else {
            self.agc_gain = 1.0;
        }

        // --- применение: гейт × AGC (интерполяция внутри кадра) × лимитер ---
        for i in 0..HOP {
            let t = i as f32 / HOP as f32;
            let g = self.gate * (self.agc_prev + t * (self.agc_gain - self.agc_prev));
            let x = out[base + i] * g;
            out[base + i] = self.limit(x);
        }
    }

    /// Пер-сэмпловый лимитер: мгновенная атака, мягкий релиз.
    fn limit(&mut self, x: f32) -> f32 {
        let a = x.abs();
        if a > self.lim_env {
            self.lim_env = a;
        } else {
            self.lim_env *= 0.99993;
            if a > self.lim_env {
                self.lim_env = a;
            }
        }
        let target = if self.lim_env > LIMITER_THR {
            LIMITER_THR / self.lim_env
        } else {
            1.0
        };
        if target < self.lim_gain {
            self.lim_gain = target; // атака мгновенная
        } else {
            self.lim_gain += 0.0005 * (target - self.lim_gain); // релиз ~30 мс
        }
        (x * self.lim_gain).clamp(-OUT_CLAMP, OUT_CLAMP)
    }

    /// DC-блокер: one-pole highpass ~19 Гц (убирает смещение дешёвых микрофонов).
    fn dc_block(&mut self, x: f32) -> f32 {
        let y = x - self.dc_x + 0.9975 * self.dc_y;
        self.dc_x = x;
        self.dc_y = y;
        y
    }
}

/* ------------------------------------------------------------------ */
/* Тесты                                                               */
/* ------------------------------------------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;

    /// Наивный DFT (O(n²), f64) — эталон для сверки FFT.
    fn dft_naive(x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = x.len();
        let mut re = vec![0f32; n];
        let mut im = vec![0f32; n];
        for k in 0..n {
            let mut sr = 0f64;
            let mut si = 0f64;
            for (t, v) in x.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (k as f64) * (t as f64) / n as f64;
                sr += *v as f64 * a.cos();
                si += *v as f64 * a.sin();
            }
            re[k] = sr as f32;
            im[k] = si as f32;
        }
        (re, im)
    }

    /// Детерминированный LCG — шум без внешних крейтов.
    struct Lcg(u64);
    impl Lcg {
        fn next_f32(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 11) as f32 / (1u64 << 53) as f32) * 2.0 - 1.0
        }
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin() * amp
            })
            .collect()
    }

    /// Речеподобный сигнал: бёрсты 300 Гц (200 мс) с паузами (400 мс),
    /// везде подложен шум.
    fn speech_bursts(secs: f32, speech_rms: f32, noise_rms: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f32) as usize;
        let mut rng = Lcg(0x5EED);
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / SAMPLE_RATE as f32;
            let noise = rng.next_f32() * noise_rms;
            let s = if t % 0.6 < 0.2 {
                (2.0 * std::f32::consts::PI * 300.0 * t).sin()
                    * speech_rms
                    * std::f32::consts::SQRT_2
                    + noise
            } else {
                noise
            };
            v.push(s.clamp(-1.0, 1.0));
        }
        v
    }

    fn rms(s: &[f32]) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        (s.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>() / s.len() as f64).sqrt()
            as f32
    }

    fn db(x: f32) -> f32 {
        20.0 * (x + 1.0e-8).log10()
    }

    fn run(dsp: &mut VoiceDsp, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for c in input.chunks(chunk.max(1)) {
            dsp.push(c, &mut out);
        }
        out
    }

    #[test]
    fn fft_matches_naive_dft() {
        let x: Vec<f32> = (0..WIN)
            .map(|i| {
                0.5 + 0.3 * (2.0 * std::f32::consts::PI * 50.0 * i as f32 / WIN as f32).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 137.0 * i as f32 / WIN as f32 + 0.7).sin()
            })
            .collect();
        let (nre, nim) = dft_naive(&x);
        let mut re = x.clone();
        let mut im = vec![0.0; WIN];
        Fft::new(WIN).forward(&mut re, &mut im);
        for k in 0..WIN {
            let tol = 0.02 * nre[k].abs().max(nim[k].abs()).max(1.0) + 0.05;
            assert!(
                (re[k] - nre[k]).abs() < tol,
                "bin {k}: re {} vs {} (tol {tol})",
                re[k],
                nre[k]
            );
            assert!(
                (im[k] - nim[k]).abs() < tol,
                "bin {k}: im {} vs {} (tol {tol})",
                im[k],
                nim[k]
            );
        }
    }

    #[test]
    fn fft_inverse_roundtrip() {
        let mut rng = Lcg(42);
        let x: Vec<f32> = (0..WIN).map(|_| rng.next_f32() * 0.8).collect();
        let mut re = x.clone();
        let mut im = vec![0.0; WIN];
        let fft = Fft::new(WIN);
        fft.forward(&mut re, &mut im);
        fft.inverse(&mut re, &mut im);
        for (i, v) in x.iter().enumerate() {
            assert!(
                (re[i] - v).abs() < 1.0e-3,
                "roundtrip {i}: {} vs {v}",
                re[i]
            );
            assert!(im[i].abs() < 1.0e-3, "roundtrip im {i}: {}", im[i]);
        }
    }

    #[test]
    fn bypass_is_transparent() {
        // всё выключено → DC-блокер + лимитер не трогают слабый сигнал
        let cfg = DspConfig {
            ns_enabled: false,
            ns_strength: NsStrength::Medium,
            agc_enabled: false,
        };
        let mut dsp = VoiceDsp::new(cfg);
        let input = sine(440.0, 0.5, SAMPLE_RATE as usize);
        let out = run(&mut dsp, &input, 480);
        assert_eq!(out.len(), input.len(), "транзит не меняет длину");
        // пропускаем переходный процесс DC-блокера (лёгкий сдвиг фазы ВЧ-фильтра)
        for (i, (o, s)) in out.iter().zip(&input).enumerate().skip(9600) {
            assert!(
                (o - s).abs() < 0.035,
                "транзит исказил сэмпл {i}: {o} vs {s}"
            );
        }
    }

    #[test]
    fn white_noise_suppressed() {
        // белый шум −30 dBFS → на «среднем» шумодаве тише минимум на 12 dB
        let mut rng = Lcg(7);
        let input: Vec<f32> = (0..2 * SAMPLE_RATE as usize)
            .map(|_| rng.next_f32() * 0.0316)
            .collect();
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let out = run(&mut dsp, &input, 480);
        let tail = &out[out.len() - SAMPLE_RATE as usize..];
        let supp = db(rms(tail));
        assert!(supp <= -42.0, "шумодав не справился: {supp} dBFS");
    }

    #[test]
    fn strength_ordering() {
        let mut rng = Lcg(7);
        let input: Vec<f32> = (0..2 * SAMPLE_RATE as usize)
            .map(|_| rng.next_f32() * 0.0316)
            .collect();
        let mut levels = Vec::new();
        for strength in [NsStrength::Low, NsStrength::Medium, NsStrength::High] {
            let cfg = DspConfig {
                ns_strength: strength,
                ..DspConfig::default()
            };
            let mut dsp = VoiceDsp::new(cfg);
            let out = run(&mut dsp, &input, 480);
            let tail = &out[out.len() - SAMPLE_RATE as usize..];
            levels.push(db(rms(tail)));
        }
        assert!(
            levels[0] > levels[1] + 4.0 && levels[1] > levels[2] + 4.0,
            "сила не монотонна: {levels:?}"
        );
    }

    #[test]
    fn ns_preserves_clean_speech() {
        // чистый тон −18 dBFS (перед ним четверть секунды тишины —
        // шумодаву нужно время спрофилировать фон): речь не душится
        let mut lead = vec![0.0; SAMPLE_RATE as usize / 4];
        lead.extend(sine(440.0, 0.126, 2 * SAMPLE_RATE as usize)); // rms ≈ −21 dBFS
        let input = lead;
        let cfg = DspConfig {
            agc_enabled: false,
            ..DspConfig::default()
        };
        let mut dsp = VoiceDsp::new(cfg);
        let out = run(&mut dsp, &input, 480);
        let tail = &out[out.len() - SAMPLE_RATE as usize..];
        let lvl = db(rms(tail));
        assert!(lvl > -27.0, "речь передавлена: {lvl} dBFS (вход −21)");
    }

    #[test]
    fn agc_boosts_quiet_speech() {
        // тихая речь −35 dBFS → усиление растёт, выход приближается к −20 dBFS
        let input = speech_bursts(5.0, 0.0178, 0.0056); // речь −35 dB, шум −45 dB
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let out = run(&mut dsp, &input, 480);
        let m = dsp.metrics();
        assert!(m.agc_db >= 10.0, "AGC не разогнался: {} dB", m.agc_db);
        assert!(m.agc_db <= 30.5, "AGC вышел за потолок: {} dB", m.agc_db);
        let tail = &out[out.len() - SAMPLE_RATE as usize..];
        let lvl = db(rms(tail));
        assert!(lvl > -32.0, "выход слишком тихий: {lvl} dBFS");
    }

    #[test]
    fn agc_tames_loud_speech() {
        // громкая речь −3 dBFS → усиление уходит вниз, клипа нет
        let input = speech_bursts(5.0, 0.7, 0.0056);
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let out = run(&mut dsp, &input, 480);
        let m = dsp.metrics();
        assert!(m.agc_db <= -10.0, "AGC не ослаб: {} dB", m.agc_db);
        let peak = out.iter().fold(0f32, |a, s| a.max(s.abs()));
        assert!(peak <= 0.995, "клип: пик {peak}");
        let tail = &out[out.len() - SAMPLE_RATE as usize..];
        let lvl = db(rms(tail));
        assert!(
            lvl > -28.0 && lvl < -12.0,
            "уровень вне коридора: {lvl} dBFS"
        );
    }

    #[test]
    fn agc_frozen_in_silence() {
        // после речи — только шум: усиление заморожено, не убегает вверх
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let mut out = Vec::new();
        let speech = speech_bursts(2.0, 0.0178, 0.0056);
        dsp.push(&speech, &mut out);
        let gain_after_speech = dsp.metrics().agc_db;
        assert!(
            gain_after_speech > 8.0,
            "после речи усиление мало: {gain_after_speech}"
        );

        let mut rng = Lcg(99);
        let noise: Vec<f32> = (0..2 * SAMPLE_RATE as usize)
            .map(|_| rng.next_f32() * 0.0056)
            .collect();
        dsp.push(&noise, &mut out);
        let m = dsp.metrics();
        assert!(
            (m.agc_db - gain_after_speech).abs() <= 2.5,
            "усиление уползло в тишине: {gain_after_speech} → {}",
            m.agc_db
        );
        assert!(!m.speech, "VAD не закрылся на чистом шуме");
    }

    #[test]
    fn limiter_no_clip() {
        // полномасштабный меандр — на выходе ни клипа, ни NaN
        let input: Vec<f32> = (0..SAMPLE_RATE as usize)
            .map(|i| if i % 240 < 120 { 1.0 } else { -1.0 })
            .collect();
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let out = run(&mut dsp, &input, 480);
        for (i, s) in out.iter().enumerate() {
            assert!(s.is_finite(), "NaN/inf на {i}");
            assert!(s.abs() <= 0.995, "клип на {i}: {s}");
        }
    }

    #[test]
    fn edge_inputs_no_nans() {
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let mut out = Vec::new();
        dsp.push(&vec![0.0; 24_000], &mut out); // тишина
        dsp.push(&vec![0.5; 24_000], &mut out); // DC
        dsp.push(&vec![-1.0; 24_000], &mut out); // постоянная −1
        dsp.push(&[], &mut out); // пустой чанк
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn chunking_does_not_matter() {
        // кадры нарезаются внутренним аккумулятором: чанки не влияют на выход
        let input = speech_bursts(2.0, 0.03, 0.006);
        let mut a = VoiceDsp::new(DspConfig::default());
        let mut b = VoiceDsp::new(DspConfig::default());
        let oa = run(&mut a, &input, 137);
        let ob = run(&mut b, &input, 960);
        assert_eq!(oa.len(), ob.len(), "длины разошлись");
        let max_diff = oa
            .iter()
            .zip(&ob)
            .fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(max_diff < 1.0e-6, "выходы разошлись на {max_diff}");
    }

    #[test]
    fn config_switch_midstream_no_panic() {
        let input = speech_bursts(2.0, 0.05, 0.01);
        let mut dsp = VoiceDsp::new(DspConfig::default());
        let mut out = Vec::new();
        for (i, c) in input.chunks(480).enumerate() {
            if i % 4 == 0 {
                dsp.set_config(DspConfig {
                    ns_enabled: false,
                    ns_strength: NsStrength::High,
                    agc_enabled: false,
                });
            } else {
                dsp.set_config(DspConfig::default());
            }
            dsp.push(c, &mut out);
        }
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn dc_offset_removed() {
        // синус + DC 0.3 → на выходе смещения нет
        let input: Vec<f32> = sine(300.0, 0.25, SAMPLE_RATE as usize)
            .into_iter()
            .map(|s| s + 0.3)
            .collect();
        let cfg = DspConfig {
            ns_enabled: false,
            ns_strength: NsStrength::Medium,
            agc_enabled: false,
        };
        let mut dsp = VoiceDsp::new(cfg);
        let out = run(&mut dsp, &input, 480);
        let tail = &out[out.len() - 12_000..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 0.02, "DC не убран: среднее {mean}");
    }
}
