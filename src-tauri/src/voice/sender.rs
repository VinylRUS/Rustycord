//! Микрофонный путь: capture → opus → RTP → xsalsa20poly1305 → UDP.
//!
//! Работает в выделенном std-потоке: колбэки cpal только складывают сэмплы,
//! кодирование (десятки микросекунд) и отправка никогда не выполняются в
//! аудио-колбэке — золотое правило real-time: колбэк не блокирует.
//!
//! VAD — простая энергетическая модель с гистерезисом (~300 мс удержания):
//! анонс op 5 SPEAKING отправляется только на переходах, ровно как это
//! делает официальный клиент с voice activity.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

use crate::voice::codec::VoiceEncoder;
use crate::voice::crypto::VoiceCrypto;
use crate::voice::udp;
use crate::voice::{VoiceControls, VoiceWsCmd};

/// Сэмплов моно-кадра при 48 кГц / 20 мс.
const FRAME_MONO: usize = 960;
/// Порог RMS для «речи» (амплитуда ~1.2% — эмпирика для бытовых микрофонов).
const VAD_THRESHOLD: f32 = 0.012;
/// Сколько кадров после последнего громкого держим speaking (300 мс).
const VAD_HOLD: u8 = 15;

pub fn spawn(
    sock: Arc<UdpSocket>,
    crypto: Arc<VoiceCrypto>,
    controls: Arc<Mutex<VoiceControls>>,
    mic_buf: Arc<Mutex<Vec<i16>>>,
    ssrc: u32,
    ws_cmd: tokio::sync::mpsc::UnboundedSender<VoiceWsCmd>,
    cancel: CancellationToken,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        run(sock, crypto, controls, mic_buf, ssrc, ws_cmd, cancel);
    })
}

#[allow(clippy::too_many_arguments)]
fn run(
    sock: Arc<UdpSocket>,
    crypto: Arc<VoiceCrypto>,
    controls: Arc<Mutex<VoiceControls>>,
    mic_buf: Arc<Mutex<Vec<i16>>>,
    ssrc: u32,
    ws_cmd: tokio::sync::mpsc::UnboundedSender<VoiceWsCmd>,
    cancel: CancellationToken,
) {
    let enc = match VoiceEncoder::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("voice: кодировщик недоступен, микрофон не работает: {e:#}");
            return;
        }
    };

    let mut seq: u16 = 0;
    let mut ts: u32 = 0;
    let mut marker = true; // первый кадр сессии — маркерный
    let mut speaking_on = false;
    let mut loud_hold: u8 = 0;
    let mut opus_out = [0u8; 512];
    let mut pkt_buf: Vec<u8> = Vec::with_capacity(600);

    tracing::info!("voice: отправитель запущен (ssrc={ssrc})");
    loop {
        if cancel.is_cancelled() {
            break;
        }
        std::thread::sleep(Duration::from_millis(4));

        // 1. Забираем ровно один кадр из аккумулятора колбэка ввода.
        let frame: Vec<i16> = {
            let mut buf = mic_buf.lock().unwrap();
            if buf.len() < FRAME_MONO {
                continue;
            }
            buf.drain(..FRAME_MONO).collect()
        };

        // 2. Мьют/деафиен: кадр выбрасывается, seq/ts не двигаются.
        if controls.lock().unwrap().mic_off() {
            if speaking_on {
                speaking_on = false;
                let _ = ws_cmd.send(VoiceWsCmd::SetSpeaking(false));
            }
            continue;
        }

        // 3. Энергетический VAD с гистерезисом.
        let rms = {
            let mut acc = 0f32;
            for s in &frame {
                let v = *s as f32 / 32768.0;
                acc += v * v;
            }
            (acc / frame.len() as f32).sqrt()
        };
        if rms > VAD_THRESHOLD {
            loud_hold = VAD_HOLD;
        } else {
            loud_hold = loud_hold.saturating_sub(1);
        }
        let now_speaking = loud_hold > 0;
        if now_speaking != speaking_on {
            speaking_on = now_speaking;
            if speaking_on {
                marker = true; // всплеск речи начинается с маркерного кадра
            }
            let _ = ws_cmd.send(VoiceWsCmd::SetSpeaking(speaking_on));
        }

        // 4. Кодируем 20 мс моно.
        let n = match enc.encode_frame(&frame, &mut opus_out) {
            Ok(n) => n,
            Err(e) => {
                tracing::debug!("voice: opus encode: {e:#}");
                continue;
            }
        };

        // 5. RTP + xsalsa20poly1305 (nonce = заголовок, tag-first).
        let header = udp::build_rtp_header(marker, seq, ts, ssrc);
        marker = false;
        let payload = match crypto.encrypt_rtp(&header, &opus_out[..n]) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("voice: encrypt: {e:#}");
                continue;
            }
        };
        pkt_buf.clear();
        pkt_buf.extend_from_slice(&header);
        pkt_buf.extend_from_slice(&payload);

        // 6. Отправка. try_send не блокирует поток; WouldBlock = потеря кадра,
        //    для голоса это правильная политика (лучше кадр, чем пауза цикла).
        if let Err(e) = sock.try_send(&pkt_buf) {
            tracing::debug!("voice: udp send: {e}");
        }
        seq = seq.wrapping_add(1);
        ts = ts.wrapping_add(FRAME_MONO as u32);
    }
    if speaking_on {
        let _ = ws_cmd.send(VoiceWsCmd::SetSpeaking(false));
    }
    tracing::info!("voice: отправитель остановлен");
}
