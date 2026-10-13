//! Voice WebSocket (media signaling) — protocol version 4.
//!
//! Handshake, reverse-engineered from the official client's traffic:
//!   1. connect  wss://{endpoint}:443/?v=4
//!   2. op 8 HELLO        → heartbeat_interval
//!   3. op 0 IDENTIFY     → server_id, user_id, session_id, token
//!   4. op 2 READY       → ip, port, ssrc, encryption modes
//!   5. UDP IP discovery  → our external address behind NAT
//!   6. op 1 SELECT_PROTOCOL (mode xsalsa20_poly1305)
//!   7. op 4 SESSION_DESCRIPTION → 32-byte secret_key
//!   8. op 5 SPEAKING (ssrc is a *string* here — a classic protocol quirk)
//!
//! Then: heartbeat op 3, speaking broadcasts op 5, client connect op 12,
//! client disconnect op 13. Audio itself travels as encrypted RTP over UDP.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::net::UdpSocket;
use tokio_tungstenite::connect_async;
use tokio_util::sync::CancellationToken;

use crate::state::{SharedState, VoiceHandshake};
use crate::voice::crypto::VoiceCrypto;
use crate::voice::udp;
use crate::wsutil;

pub async fn run_voice(
    app: tauri::AppHandle,
    state: SharedState,
    hs: VoiceHandshake,
    cancel: CancellationToken,
) {
    tracing::info!("voice: запускаем сессию для гильдии {}", hs.guild_id);
    if let Err(e) = voice_session(&app, &hs, &cancel).await {
        tracing::error!("voice: ошибка сессии: {e:#}");
        let _ = app.emit(
            "discord://voice",
            json!({ "kind": "disconnected", "data": { "guild_id": hs.guild_id } }),
        );
    }
    // Self-cleanup so a fresh join can start immediately.
    state.voice_tasks.lock().await.remove(&hs.guild_id);
    tracing::info!("voice: сессия {} завершена", hs.guild_id);
}

async fn voice_session(
    app: &tauri::AppHandle,
    hs: &VoiceHandshake,
    cancel: &CancellationToken,
) -> Result<()> {
    let endpoint = hs.endpoint.clone().context("нет endpoint")?;
    let session_id = hs.session_id.clone().context("нет session_id")?;
    let token = hs.token.clone().context("нет voice token")?;

    // ---- 1. connect ----
    let url = format!("wss://{endpoint}:443/?v=4");
    let (ws, _) = connect_async(url).await.context("voice ws connect")?;
    let (mut writer, mut reader) = wsutil::split(ws);

    // ---- 2. HELLO ----
    let hello = wsutil::next_json(&mut reader).await?;
    if hello["op"].as_u64() != Some(8) {
        bail!("ожидали voice HELLO op 8, получили {:?}", hello["op"]);
    }
    let hb_interval = hello["d"]["heartbeat_interval"]
        .as_u64()
        .unwrap_or(5_000)
        .clamp(1000, 60_000);

    // ---- 3. IDENTIFY ----
    wsutil::send_json(
        &mut writer,
        &json!({
            "op": 0,
            "d": {
                "server_id": hs.guild_id,   // yes, "server_id" = guild — legacy naming
                "user_id": hs.user_id,
                "session_id": session_id,
                "token": token
            }
        }),
    )
    .await?;

    // ---- 4. READY ----
    let ready = loop {
        let v = wsutil::next_json(&mut reader).await?;
        if cancel.is_cancelled() {
            let _ = writer.close().await;
            return Ok(());
        }
        if v["op"].as_u64() == Some(2) {
            break v["d"].clone();
        }
    };
    let media_ip = ready["ip"]
        .as_str()
        .context("voice READY без ip")?
        .to_string();
    let media_port = ready["port"].as_u64().context("voice READY без port")? as u16;
    let ssrc = ready["ssrc"].as_u64().context("voice READY без ssrc")? as u32;
    let modes: Vec<String> = serde_json::from_value(ready["modes"].clone()).unwrap_or_default();
    let mode = if modes.iter().any(|m| m == "xsalsa20_poly1305") {
        "xsalsa20_poly1305".to_string()
    } else {
        modes
            .first()
            .cloned()
            .unwrap_or_else(|| "xsalsa20_poly1305".into())
    };
    tracing::info!("voice: READY ssrc={ssrc} media={media_ip}:{media_port} mode={mode}");

    // ---- 5. UDP IP discovery (NAT traversal) ----
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    sock.connect((media_ip.as_str(), media_port)).await?;
    let (my_ip, my_port) = udp::discover(&sock, ssrc).await?;
    tracing::info!("voice: NAT discovery → {my_ip}:{my_port}");

    // ---- 6. SELECT_PROTOCOL ----
    wsutil::send_json(
        &mut writer,
        &json!({
            "op": 1,
            "d": {
                "protocol": "udp",
                "data": { "address": my_ip, "port": my_port, "mode": mode }
            }
        }),
    )
    .await?;

    // ---- 7. SESSION_DESCRIPTION → secret key ----
    let sd = loop {
        let v = wsutil::next_json(&mut reader).await?;
        match v["op"].as_u64() {
            Some(4) => break v["d"].clone(),
            Some(5) | Some(12) | Some(13) => continue, // события до готовности — игнорируем
            _ => continue,
        }
    };
    let secret_key: Vec<u8> =
        serde_json::from_value(sd["secret_key"].clone()).context("secret_key")?;
    anyhow::ensure!(
        secret_key.len() == 32,
        "ожидали 32-байтный ключ, получили {}",
        secret_key.len()
    );
    let crypto = Arc::new(VoiceCrypto::new(&secret_key).context("crypto init")?);

    // ssrc → user_id, shared between the WS loop (op 12/13) and the RTP reader.
    let ssrc_map: Arc<Mutex<HashMap<u32, String>>> = Arc::new(Mutex::new(HashMap::new()));

    // ---- 8. SPEAKING (ssrc as string — известная странность протокола) ----
    wsutil::send_json(
        &mut writer,
        &json!({ "op": 5, "d": { "speaking": 1, "delay": 0, "ssrc": ssrc.to_string() } }),
    )
    .await?;

    let _ = app.emit(
        "discord://voice",
        json!({
            "kind": "connected",
            "data": {
                "guild_id": hs.guild_id,
                "mode": mode,
                "ssrc": ssrc,
                "endpoint": endpoint,
                "ip": my_ip,
                "port": my_port
            }
        }),
    );

    // ---- UDP: keepalive + RTP sniffing ----
    let sock = Arc::new(sock);
    {
        let sock = Arc::clone(&sock);
        let cancel = cancel.clone();
        tauri::async_runtime::spawn(async move {
            let ka = udp::keepalive_packet(ssrc);
            let mut t = tokio::time::interval(Duration::from_secs(5));
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            t.tick().await;
            while !cancel.is_cancelled() {
                let _ = sock.send(&ka).await;
                t.tick().await;
            }
        });
    }
    {
        let sock = Arc::clone(&sock);
        let cancel = cancel.clone();
        let crypto = Arc::clone(&crypto);
        let ssrc_map = Arc::clone(&ssrc_map);
        tauri::async_runtime::spawn(async move {
            let mut buf = [0u8; 1500];
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    res = sock.recv(&mut buf) => {
                        let Ok(n) = res else { break };
                        let Some(h) = udp::parse_rtp_header(&buf[..n]) else { continue };
                        // Расшифровываем кадр (nonce = RTP-заголовок) — пайплайн
                        // проверен здесь; вывод opus на динамики — в роадмапе.
                        match crypto.decrypt_rtp(&buf[..h.header_len], &buf[h.header_len..n]) {
                            Ok(plain) => {
                                let who = ssrc_map.lock().unwrap().get(&h.ssrc).cloned();
                                tracing::debug!(
                                    ssrc = h.ssrc,
                                    seq = h.sequence,
                                    ts = h.timestamp,
                                    marker = h.marker,
                                    pt = h.payload_type,
                                    opus_bytes = plain.len(),
                                    user = who.as_deref().unwrap_or("?"),
                                    "voice: opus frame decrypted"
                                );
                            }
                            Err(e) => tracing::debug!(ssrc = h.ssrc, error = %e, "voice: bad rtp frame"),
                        }
                    }
                }
            }
        });
    }

    // ---- main loop: heartbeat + speaking events ----
    let mut hb = tokio::time::interval(Duration::from_millis(hb_interval));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    hb.tick().await;

    // ssrc → user_id map, populated by op 12 "Client Connected".

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = writer.close().await;
                break Ok(());
            }
            _ = hb.tick() => {
                wsutil::send_json(&mut writer, &json!({
                    "op": 3, "d": chrono::Utc::now().timestamp_millis()
                })).await?;
            }
            msg = reader.next() => {
                let Some(msg) = msg else { break Ok(()) };
                let msg = msg.context("voice ws ошибка")?;
                let text = match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text,
                    tokio_tungstenite::tungstenite::Message::Close(_) => break Ok(()),
                    _ => continue,
                };
                let v: Value = serde_json::from_str(&text)?;
                match v["op"].as_u64().unwrap_or(u64::MAX) {
                    5 => {
                        // Speaking: {speaking, delay, ssrc} — ssrc бывает и числом, и строкой.
                        let ssrc_val = v["d"]["ssrc"].as_u64()
                            .or_else(|| v["d"]["ssrc"].as_str().and_then(|s| s.parse().ok()));
                        let speaking = v["d"]["speaking"].as_i64().unwrap_or(0) != 0;
                        if let Some(ss) = ssrc_val {
                            if let Some(uid) = ssrc_map.lock().unwrap().get(&(ss as u32)).cloned() {
                                let _ = app.emit("discord://voice", json!({
                                    "kind": "speaking",
                                    "data": { "guild_id": hs.guild_id, "user_id": uid, "speaking": speaking }
                                }));
                            }
                        }
                    }
                    12 => {
                        let uid = v["d"]["user_id"].as_str().unwrap_or_default().to_string();
                        let audio_ssrc = v["d"]["audio_ssrc"].as_u64().unwrap_or(0) as u32;
                        if !uid.is_empty() && audio_ssrc != 0 {
                            tracing::debug!("voice: user {uid} подключился (ssrc {audio_ssrc})");
                            ssrc_map.lock().unwrap().insert(audio_ssrc, uid);
                        }
                    }
                    13 => {
                        let uid = v["d"]["user_id"].as_str().unwrap_or_default().to_string();
                        if let Some(k) = ssrc_map.lock().unwrap().iter().find(|(_, u)| **u == uid).map(|(k, _)| *k) {
                            ssrc_map.lock().unwrap().remove(&k);
                        }
                        let _ = app.emit("discord://voice", json!({
                            "kind": "speaking",
                            "data": { "guild_id": hs.guild_id, "user_id": uid, "speaking": false }
                        }));
                    }
                    _ => {}
                }
            }
        }
    }
}
