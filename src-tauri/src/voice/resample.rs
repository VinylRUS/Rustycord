//! Линейный ресемплер — стыкует произвольную частоту устройства с 48 кГц
//! движка. Хранит непотреблённые входные сэмплы и дробную позицию между
//! ними; `push` отдаёт всё, что уже можно интерполировать.
//!
//! Качественно этого достаточно для голоса: A/B сравнение слепым слухом
//! линейного vs синкового ресемплера на 48→44.1 в речевой полосе —
//! неразличимо на бытовом железе.

/// Инкрементальный линейный ресемплер, интерлив-каналы.
pub struct LinearResampler {
    step: f64, // входных кадров на один выходной
    channels: usize,
    /// Дробный индекс следующего выходного кадра во входном потоке.
    pos: f64,
    /// Непотреблённые входные кадры (интерлив), начиная с floor(pos).
    stash: Vec<f32>,
}

impl LinearResampler {
    /// Пан-through при равных частотах не создаём вовсе (проверяет вызывающий).
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        debug_assert!(channels >= 1);
        Self {
            step: f64::from(in_rate) / f64::from(out_rate),
            channels,
            pos: 0.0,
            stash: Vec::new(),
        }
    }

    pub fn is_passthrough(&self) -> bool {
        (self.step - 1.0).abs() < 1e-12
    }

    /// Скармливает chunk (интерлив) и возвращает готовые выходные сэмплы.
    /// Возвращённый вектор может быть короче «справедливой» доли —
    /// остаток доинтерполируется следующими push (это поток, а не файл).
    pub fn push(&mut self, chunk: &[f32]) -> Vec<f32> {
        self.stash.extend_from_slice(chunk);
        let ch = self.channels;
        let mut out = Vec::new();
        // Нужны кадры floor(pos) и floor(pos)+1 — т.е. как минимум 2 в stash.
        loop {
            let i0 = self.pos.floor() as usize;
            if i0 + 1 >= self.stash.len() / ch {
                break;
            }
            let frac = self.pos - i0 as f64;
            for c in 0..ch {
                let l = self.stash[i0 * ch + c];
                let r = self.stash[(i0 + 1) * ch + c];
                out.push(l + (r - l) * frac as f32);
            }
            self.pos += self.step;
            if self.pos >= 1.0e9 {
                // страховка от дрейфа дробной позиции на бесконечных потоках
                let whole = self.pos.floor();
                let drop = whole as usize * ch;
                self.stash.drain(..drop);
                self.pos -= whole;
                break;
            }
        }
        // Съедаем всё, что осталось слева от floor(pos).
        let keep_from = (self.pos.floor() as usize).min(self.stash.len() / ch);
        if keep_from > 0 {
            self.stash.drain(..keep_from * ch);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_ratio_and_content() {
        // 48k моно → 16k моно: 10 мс → ~160 сэмплов, значения близки к входным
        let mut r = LinearResampler::new(48_000, 16_000, 1);
        let input: Vec<f32> = (0..480).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let out = r.push(&input);
        assert!(
            (out.len() as i64 - 160).abs() <= 2,
            "ожидали ~160 сэмплов, получили {}",
            out.len()
        );
        // выходные точки совпадают с входными в тех же позициях времени
        for (k, s) in out.iter().enumerate() {
            let ref_in = input[(k * 3).min(479)];
            assert!((s - ref_in).abs() < 0.06, "выход {k} сильно разошёлся");
        }
    }

    #[test]
    fn upsample_ratio() {
        let mut r = LinearResampler::new(16_000, 48_000, 1);
        let input: Vec<f32> = (0..160).map(|i| i as f32 / 160.0).collect();
        let out = r.push(&input);
        assert!(
            (out.len() as i64 - 480 + 3).abs() <= 6,
            "ожидали ~477 сэмплов, получили {}",
            out.len()
        );
        // монотонный растущий сигнал должен остаться монотонным
        for w in out.windows(2) {
            assert!(w[1] >= w[0] - 1e-6, "нарушение монотонности");
        }
    }

    #[test]
    fn stereo_channels_stay_paired() {
        // Левый = +0.5, правый = -0.5 на 48k → 24k: пары должны сохраниться
        let mut r = LinearResampler::new(48_000, 24_000, 2);
        let mut input = Vec::new();
        for _ in 0..480 {
            input.push(0.5);
            input.push(-0.5);
        }
        let out = r.push(&input);
        assert_eq!(out.len() % 2, 0);
        for pair in out.chunks(2) {
            assert!((pair[0] - 0.5).abs() < 1e-4);
            assert!((pair[1] + 0.5).abs() < 1e-4);
        }
    }
}
