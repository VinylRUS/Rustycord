//! Jitter-буфер приёма: переупорядочивание, дедупликация и компенсация
//! потерь RTP-кадров, плюс микширование нескольких говорящих.
//!
//! Дизайн сознательно простой — это голос, а не AVB-студия:
//! каждое SSRC получает свою дорожку (`Channel`) с очередью сэмплов;
//! вывод cpal раз в колбэк забирает по сэмплу из каждой дорожки
//! (`Mixer::mix_into`). Правила дорожки:
//!
//! * кадры лежат в `pending` по seq и выезжают в очередь строго по порядку;
//! * опоздавшие вглубь прошлого (seq < expected) — отбрасываются навсегда;
//! * дыра ≤ 8 кадров (~160 мс) — заполняется одним кадром тишины за шаг;
//! * дыра > 8 кадров — ресинк без тишины (латентность важнее выравнивания);
//! * перед стартом дорожка копит пре-ролл 40 мс — окно на переупорядочивание;
//! * очередь ограничена 1 с (96 000 стерео-сэмплов) и 64 кадрами lookahead.

use std::collections::{BTreeMap, HashMap, VecDeque};

/// 20 мс стерео @ 48 кГц — размер одного кадра для расчёта тишины.
const FRAME_SAMPLES: usize = 1920;
/// Потолок очереди дорожки: 1 секунда.
const CHANNEL_CAP: usize = 96_000;
/// Пре-ролл: сколько данных копим, прежде чем дорожка начинает звучать.
const PRIME_SAMPLES: usize = 3840; // 40 мс
/// Максимум кадров в lookahead-карте.
const PENDING_CAP: usize = 64;

#[derive(Default)]
struct Channel {
    /// Следующий ожидаемый seq (устанавливается первым пришедшим кадром).
    expected: Option<u16>,
    /// Кадры из «будущего» до прихода ожидаемого.
    pending: BTreeMap<u16, Vec<f32>>,
    /// Готовые к выводу сэмплы (стерео-интерлив), в порядке поступления.
    q: VecDeque<f32>,
    /// Дорожка проигралась и опустела — следующий всплеск заново копит пре-ролл.
    primed: bool,
}

impl Channel {
    /// Один сэмпл дорожки (0.0 = тишина, если данных нет).
    fn next_sample(&mut self) -> f32 {
        self.service();
        if !self.primed {
            if self.q.len() < PRIME_SAMPLES {
                return 0.0;
            }
            self.primed = true;
        }
        match self.q.pop_front() {
            Some(v) => v,
            None => {
                self.primed = false;
                0.0
            }
        }
    }

    /// Продвигает очередь: drain по порядку, компенсация потерь, ресинки.
    /// Идемпотентно — вызывается на каждый next_sample, обычно делает O(1).
    fn service(&mut self) {
        loop {
            self.drain();
            let Some(exp) = self.expected else {
                return;
            };
            if self.q.len() >= PRIME_SAMPLES {
                return;
            }
            let Some(next) = self.pending.keys().next().copied() else {
                return; // будущего нет — просто ждём сеть
            };
            let gap = next.wrapping_sub(exp);
            if gap > 0 && gap <= 8 {
                // короткая потеря: один кадр тишины, считаем его потерянным
                self.q.extend(std::iter::repeat(0.0).take(FRAME_SAMPLES));
                self.expected = Some(exp.wrapping_add(1));
            } else {
                // дыра > 8 кадров или новый всплеск после паузы — ресинк
                self.expected = Some(next);
            }
        }
    }

    /// Всё, что совпадает с expected, переносится из pending в очередь.
    fn drain(&mut self) {
        while let Some(e) = self.expected {
            let Some(f) = self.pending.remove(&e) else {
                break;
            };
            self.q.extend(f);
            self.expected = Some(e.wrapping_add(1));
        }
    }

    fn on_frame(&mut self, seq: u16, frame: Vec<f32>) {
        if self.expected.is_none() {
            self.expected = Some(seq);
        }
        // Пакет из прошлого — опоздал безвозвратно.
        if let Some(exp) = self.expected {
            if seq.wrapping_sub(exp) >= 0x8000 {
                return;
            }
        }
        self.pending.insert(seq, frame);
        self.drain();
        while self.pending.len() > PENDING_CAP {
            let oldest = *self.pending.keys().next().expect("pending non-empty");
            self.pending.remove(&oldest);
        }
        if self.q.len() > CHANNEL_CAP {
            let drop = self.q.len() - CHANNEL_CAP;
            self.q.drain(..drop);
        }
    }
}

/// Микшер голосовой сессии: ssrc → дорожка.
#[derive(Default)]
pub struct Mixer {
    channels: HashMap<u32, Channel>,
}

impl Mixer {
    /// Принять декодированный кадр (стерео-интерлив f32) от говорящего.
    pub fn on_frame(&mut self, ssrc: u32, seq: u16, frame: Vec<f32>) {
        self.channels.entry(ssrc).or_default().on_frame(seq, frame);
    }

    /// Убрать дорожку (участник отключился — op 13).
    pub fn remove_channel(&mut self, ssrc: u32) {
        self.channels.remove(&ssrc);
    }

    /// Суммирует все дорожки в `out` (интерлив стерео 48 кГц) и клампит [-1; 1].
    /// При `deafened` — тишина (собственный деафиен).
    pub fn mix_into(&mut self, out: &mut [f32], deafened: bool) {
        if deafened || self.channels.is_empty() {
            out.fill(0.0);
            return;
        }
        out.fill(0.0);
        for ch in self.channels.values_mut() {
            for o in out.iter_mut() {
                *o += ch.next_sample();
            }
        }
        for o in out.iter_mut() {
            *o = o.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Одна дорожка (ssrc = 7), кадр = FRAME_SAMPLES сэмплов с меткой `tag`.
    fn frame(tag: f32) -> Vec<f32> {
        vec![tag; FRAME_SAMPLES]
    }

    /// Вычитает из микшера n сэмплов и возвращает их.
    fn drain_samples(mixer: &mut Mixer, n: usize) -> Vec<f32> {
        let mut buf = vec![f32::NAN; n];
        mixer.mix_into(&mut buf, false);
        buf
    }

    fn seg(out: &[f32], i: usize) -> &[f32] {
        &out[i * FRAME_SAMPLES..(i + 1) * FRAME_SAMPLES]
    }

    fn assert_seg(out: &[f32], i: usize, tag: f32) {
        let s = seg(out, i);
        assert!(
            s.iter().all(|s| *s == tag),
            "кадр #{i}: ожидали {tag}, получили [{}, .., {}]",
            s.first().copied().unwrap_or(f32::NAN),
            s.last().copied().unwrap_or(f32::NAN)
        );
    }

    #[test]
    fn in_order_playback_after_preroll() {
        let mut m = Mixer::default();
        m.on_frame(7, 1, frame(0.5));
        // Один кадр (20 мс) меньше пре-ролла 40 мс — дорожка молчит.
        let pre = drain_samples(&mut m, 64);
        assert!(pre.iter().all(|s| *s == 0.0), "пока < 40 мс — молчим");
        m.on_frame(7, 2, frame(0.25));
        m.on_frame(7, 3, frame(0.75));
        // Порог взят — играем строго по порядку: 1 → 2 → 3.
        let out = drain_samples(&mut m, FRAME_SAMPLES * 3);
        assert_seg(&out, 0, 0.5);
        assert_seg(&out, 1, 0.25);
        assert_seg(&out, 2, 0.75);
    }

    #[test]
    fn mid_stream_reorder_is_fixed_by_pending() {
        let mut m = Mixer::default();
        m.on_frame(7, 1, frame(0.1));
        m.on_frame(7, 2, frame(0.2));
        m.on_frame(7, 3, frame(0.3));
        m.on_frame(7, 4, frame(0.4));
        m.on_frame(7, 6, frame(0.6)); // кадр 5 «потерян», 6 приехал раньше него
        m.on_frame(7, 5, frame(0.5)); // 5 опоздал, но его ещё ждут — не прошлое
        let out = drain_samples(&mut m, FRAME_SAMPLES * 6);
        assert_seg(&out, 0, 0.1);
        assert_seg(&out, 1, 0.2);
        assert_seg(&out, 2, 0.3);
        assert_seg(&out, 3, 0.4);
        assert_seg(&out, 4, 0.5); // 5 сыгран до 6, несмотря на поздний приход
        assert_seg(&out, 5, 0.6);
    }

    #[test]
    fn short_gap_filled_with_silence() {
        let mut m = Mixer::default();
        m.on_frame(7, 1, frame(0.9));
        m.on_frame(7, 3, frame(0.3)); // кадр 2 потерян навсегда
        let out = drain_samples(&mut m, FRAME_SAMPLES * 3);
        assert_seg(&out, 0, 0.9);
        assert_seg(&out, 1, 0.0); // потеря = один кадр тишины
        assert_seg(&out, 2, 0.3);
    }

    #[test]
    fn far_future_resyncs_without_silence() {
        let mut m = Mixer::default();
        m.on_frame(7, 1, frame(0.9));
        m.on_frame(7, 100, frame(0.1)); // дыра в 99 кадров
        m.on_frame(7, 101, frame(0.2));
        let out = drain_samples(&mut m, FRAME_SAMPLES * 3);
        assert_seg(&out, 0, 0.9);
        // пропущенные кадры 2..99 НЕ забиваются двумя секундами тишины —
        // сразу играем 100-й (ресинк).
        assert_seg(&out, 1, 0.1);
        assert_seg(&out, 2, 0.2);
    }

    #[test]
    fn duplicates_and_past_packets_dropped() {
        let mut m = Mixer::default();
        m.on_frame(7, 7, frame(0.4));
        m.on_frame(7, 7, frame(0.4)); // дубль
        m.on_frame(7, 8, frame(0.8));
        let out = drain_samples(&mut m, FRAME_SAMPLES * 3);
        assert_seg(&out, 0, 0.4);
        assert_seg(&out, 1, 0.8);
        assert_seg(&out, 2, 0.0); // дубль не играет второй раз
    }

    #[test]
    fn queue_is_capped() {
        let mut m = Mixer::default();
        for seq in 0..120u16 {
            m.on_frame(7, seq, frame(1.0));
        }
        // 120 кадров = 230 400 сэмплов > CAP 96 000: хвост отбрасывается
        let mut total = 0usize;
        let mut buf = vec![0f32; 4096];
        loop {
            m.mix_into(&mut buf, false);
            if buf.iter().all(|s| *s == 0.0) {
                break;
            }
            total += 4096;
            assert!(total <= CHANNEL_CAP + 8192, "очередь не ограничена");
        }
    }

    #[test]
    fn deafen_mutes_output() {
        let mut m = Mixer::default();
        m.on_frame(7, 1, frame(0.8));
        let mut buf = vec![f32::NAN; FRAME_SAMPLES];
        m.mix_into(&mut buf, true);
        assert!(buf.iter().all(|s| *s == 0.0));
    }
}
