//! Тонкие обёртки над audiopus (libopus 1.5).
//!
//! Дискорд-войс — это всегда opus 48 кГц; мы кодируем моно (как большинство
//! клиентов), декодируем стерео: libopus сам апмиксит моно-потоки, так что
//! буфер вывода всегда имеет фиксированную раскладку 2×48k.

use anyhow::{Context, Result};

use audiopus::coder::{Decoder, Encoder};
use audiopus::packet::Packet;
use audiopus::{Application, Channels, MutSignals, SampleRate};

/// Кодировщик микрофона: моно 48 кГц, режим Voip.
pub struct VoiceEncoder(Encoder);

impl VoiceEncoder {
    pub fn new() -> Result<Self> {
        let enc = Encoder::new(SampleRate::Hz48000, Channels::Mono, Application::Voip)
            .context("opus: не удалось создать кодировщик")?;
        Ok(Self(enc))
    }

    /// Кодирует кадр PCM (интерлив моно); возвращает размер opus-пакета.
    pub fn encode_frame(&self, pcm: &[i16], out: &mut [u8]) -> Result<usize> {
        self.0.encode(pcm, out).context("opus encode")
    }
}

/// Декодировщик приёма: стерео 48 кГц.
pub struct VoiceDecoder(Decoder);

impl VoiceDecoder {
    pub fn new() -> Result<Self> {
        let dec = Decoder::new(SampleRate::Hz48000, Channels::Stereo)
            .context("opus: не удалось создать декодировщик")?;
        Ok(Self(dec))
    }

    /// Декодирует opus-пакет в `out` (стерео-интерлив).
    /// Возвращает число сэмплов **на канал** (для 20 мс — 960).
    pub fn decode_packet(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize> {
        if packet.is_empty() {
            anyhow::bail!("opus: пустой пакет");
        }
        let pkt = Packet::try_from(packet).map_err(|e| anyhow::anyhow!("opus packet: {e}"))?;
        let sig = MutSignals::try_from(out).map_err(|e| anyhow::anyhow!("opus signals: {e}"))?;
        let n = self
            .0
            .decode(Some(pkt), sig, false)
            .context("opus decode")?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Кодируем 5 кадров непрерывного синуса, декодируем стерео-декодером.
    /// Первый кадр у opus всегда с переходными искажениями (lookahead),
    /// поэтому сравниваем энергию по последнему кадру (steady state).
    #[test]
    fn opus_roundtrip_mono_to_stereo() {
        let enc = VoiceEncoder::new().unwrap();
        let mut dec = VoiceDecoder::new().unwrap();

        let mut pcm = [0i16; 960];
        let mut pkt = [0u8; 512];
        let mut out = [0i16; 5760 * 2];
        let to_f32 = |s: &i16| *s as f32 / 32768.0;

        let mut last_in = 0f32;
        let mut last_left = 0f32;
        let mut last_right = 0f32;

        for k in 0..5u32 {
            let phase = k as f32 * 960.0 * 0.08;
            for (i, s) in pcm.iter_mut().enumerate() {
                *s = ((i as f32 * 0.08 + phase).sin() * 9000.0) as i16;
            }
            let n = enc.encode_frame(&pcm, &mut pkt).unwrap();
            assert!(n > 8 && n < 400, "странный размер пакета: {n}");
            let per_ch = dec.decode_packet(&pkt[..n], &mut out).unwrap();
            assert_eq!(per_ch, 960, "ожидали 20 мс на канал, получили {per_ch}");
            if k == 4 {
                // Интерлив: левый = чётные, правый = нечётные.
                let n2 = per_ch * 2;
                last_in = pcm.iter().map(to_f32).map(|v| v * v).sum();
                last_left = out[0..n2]
                    .iter()
                    .step_by(2)
                    .map(to_f32)
                    .map(|v| v * v)
                    .sum();
                last_right = out[1..n2]
                    .iter()
                    .step_by(2)
                    .map(to_f32)
                    .map(|v| v * v)
                    .sum();
            }
        }

        // Апмикс моно → стерео: каналы идентичны.
        assert!(
            (last_left - last_right).abs() < 1e-6,
            "каналы стерео должны совпадать"
        );
        // Opus — lossy: в steady state требуем сохранение энергии в пределах 20%.
        assert!(
            (last_left - last_in).abs() / last_in < 0.2,
            "энергия разъехалась: in={last_in:.4} out={last_left:.4}"
        );
    }
}
