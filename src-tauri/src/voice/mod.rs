pub mod codec;
pub mod coordinator;
pub mod crypto;
// jitter и resample: в headless-сборке (без фичи `audio`) у частей вывода
// нет потребителя — приглушаем dead_code только для этого режима; в полной
// сборке (CI, релизы) предупреждения остаются включёнными.
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
pub mod jitter;
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
pub mod resample;
pub mod sender;
pub mod udp;
pub mod ws;

#[cfg(feature = "audio")]
pub mod devices;

/// Текущее состояние собственного мьюта/деафиена. Обновляется по эхо
/// VOICE_STATE_UPDATE (events.rs) — ровно как настоящий клиент узнаёт о
/// собственных флагах, — а аудиодвижок перечитывает его каждый кадр.
#[derive(Debug, Clone, Copy)]
pub struct VoiceControls {
    pub self_mute: bool,
    pub self_deaf: bool,
}

impl VoiceControls {
    pub fn new(self_mute: bool, self_deaf: bool) -> Self {
        Self {
            self_mute,
            self_deaf,
        }
    }

    /// Мьют «эффективный»: деафиен в Discord всегда глушит и микрофон.
    pub fn mic_off(&self) -> bool {
        self.self_mute || self.self_deaf
    }
}

/// Команды в главный цикл голосового WS (отправитель аудио → сигналинг).
#[derive(Debug, Clone, Copy)]
pub enum VoiceWsCmd {
    /// op 5 SPEAKING — анонсировать/снять передачу (рисует зелёные кольца
    /// у собеседников; ssrc в этом кадре, как известно, строка).
    SetSpeaking(bool),
}
