//! Голосовой движок на songbird.
//!
//! Схема: gateway (op 4) → пара событий VOICE_STATE_UPDATE +
//! VOICE_SERVER_UPDATE (events.rs собирает их в `VoiceHandshake`) →
//! координатор поднимает сессию: `songbird::Driver` сам делает весь
//! рукопожатие v4/v5 (IDENTIFY → UDP discovery → SELECT_PROTOCOL →
//! SESSION_DESCRIPTION), современное шифрование (aead_aes256_gcm /
//! xchacha20), DAVE-совместимость, jitter-буфер приёма и opus-кодек.
//!
//! Микрофон: cpal → `MicTap` → songbird Input (см. mic.rs).
//! Приём: `CoreEvent::VoiceTick` → декодированный PCM → наш `Mixer`
//! (jitter/прайминг на дорожках) → cpal-вывод (devices.rs).

pub mod coordinator;
/// DSP микрофона: шумоподавление + AGC (чистый Rust, без фичи `audio` —
/// тесты гоняются и в headless-CI).
// В headless-сборке (без фичи `audio`) потребителя нет (devices.rs не
// компилируется) — приглушаем dead_code; в полной сборке предупреждения
// остаются включёнными.
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
pub mod dsp;
// jitter и resample: в headless-сборке (без фичи `audio`) у частей вывода
// нет потребителя — приглушаем dead_code только для этого режима; в полной
// сборке (CI, релизы) предупреждения остаются включёнными.
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
pub mod jitter;
pub mod mic;
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
pub mod resample;

#[cfg(feature = "audio")]
pub mod devices;

/// Текущее состояние собственного мьюта/деафиена. Обновляется по эхо
/// VOICE_STATE_UPDATE (events.rs) — ровно как настоящий клиент узнаёт о
/// собственных флагах, — вывод (devices.rs) глушится по self_deaf, а
/// микрофон — через driver.mute в events.rs.
#[derive(Debug, Clone, Copy)]
pub struct VoiceControls {
    /// Пишется эхом op4; сам мьют микрофона применяется в songbird
    /// (events.rs: driver.mute), поэтому поле только хранит состояние.
    #[allow(dead_code)]
    pub self_mute: bool,
    /// Читается выводом (devices.rs) только в полной сборке с `audio`.
    #[cfg_attr(not(feature = "audio"), allow(dead_code))]
    pub self_deaf: bool,
}

impl VoiceControls {
    pub fn new(self_mute: bool, self_deaf: bool) -> Self {
        Self {
            self_mute,
            self_deaf,
        }
    }
}
