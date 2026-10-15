//! The Discord Gateway client — a single WebSocket to wss://gateway.discord.gg
//! that carries every real-time event, driven by a small opcode machine:
//!
//! op 0 DISPATCH · op 1 HEARTBEAT · op 2 IDENTIFY · op 4 VOICE_STATE_UPDATE
//! op 6 RESUME · op 7 RECONNECT · op 8 REQUEST_GUILD_MEMBERS
//! op 9 INVALID_SESSION · op 10 HELLO · op 11 HEARTBEAT_ACK

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::gateway::events::{handle_dispatch, SessionData};
use crate::state::{GatewayCommand, SharedState, INTENTS_FALLBACK};
use crate::wsutil;

/// capabilities для юзер-сессии — битовая маска клиентских возможностей,
/// которую шлёт официальный веб-клиент (без интентов). Значение из
/// discord-client-rs: «лучшее, если нужны все события».
const USER_CAPABILITIES: u32 = 53_607_934;

/// Properties-набор как у браузерного клиента discord.com/app — совпадает
/// с нашим X-Super-Properties, чтобы сессия выглядела консистентно.
fn client_properties(build: u64) -> serde_json::Value {
    json!({
        "os": "Windows",
        "browser": "Chrome",
        "device": "",
        "system_locale": "ru-RU",
        "browser_user_agent": crate::superprops::BROWSER_UA,
        "browser_version": crate::superprops::BROWSER_VERSION,
        "os_version": "10",
        "referrer": "",
        "referring_domain": "",
        "referrer_current": "",
        "referring_domain_current": "",
        "release_channel": "stable",
        "client_build_number": build,
        "client_event_source": null,
        "design_id": 0
    })
}

/// How the current session ended.
enum End {
    /// Clean shutdown requested by the UI.
    Shutdown,
    /// Connection dropped / server asked to reconnect — try to RESUME.
    Reconnect,
    /// Unrecoverable error (bad token, etc.) — stop the loop.
    Fatal(String),
}

/// Entry point: reconnect loop with exponential backoff (1s → 30s).
/// `session` survives across iterations so a RESUME picks up where we left.
pub async fn run_gateway(
    app: tauri::AppHandle,
    state: SharedState,
    mut rx: mpsc::Receiver<GatewayCommand>,
    shutdown: CancellationToken,
) {
    let mut session = SessionData::default();
    let mut backoff = 1u64;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        if session.id.is_some() {
            state.emit_status("reconnecting", None);
        } else {
            state.emit_status("connecting", None);
        }
        match run_session(&app, &state, &mut rx, &mut session, &shutdown).await {
            Ok(End::Shutdown) => break,
            Ok(End::Reconnect) => {
                tracing::info!(
                    "gateway: сессия прервана, возобновляем (resume={})",
                    session.id.is_some()
                );
            }
            Ok(End::Fatal(msg)) => {
                tracing::error!("gateway: фатальная ошибка: {msg}");
                state.emit_status("disconnected", Some(msg));
                break;
            }
            Err(e) => {
                tracing::error!("gateway: ошибка сессии: {e:#}");
                state.emit_status("disconnected", Some(format!("{e:#}")));
            }
        }
        if shutdown.is_cancelled() {
            break;
        }
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
        }
        backoff = (backoff * 2).min(30);
    }
    state.emit_status("shutdown", None);
    tracing::info!("gateway: остановлен");
}

async fn run_session(
    app: &tauri::AppHandle,
    state: &SharedState,
    rx: &mut mpsc::Receiver<GatewayCommand>,
    session: &mut SessionData,
    shutdown: &CancellationToken,
) -> Result<End> {
    let base = state
        .gateway_url
        .read()
        .unwrap()
        .trim_end_matches('/')
        .to_string();
    // Версия протокола: юзер-сессии — v9 (ровно как официальный веб-клиент
    // и все self-hosted реализации; v10 — только бот-режим). RESUME обязан
    // идти на той же версии, что и исходный IDENTIFY.
    let ver = if state.is_bot.load(Ordering::SeqCst) {
        10
    } else {
        9
    };
    // READY hands us a dedicated resume_gateway_url — using it is the
    // documented (and reverse-engineered) way to resume safely.
    let url = match (&session.id, &session.resume_url) {
        (Some(_), Some(resume)) => format!("{resume}/?v={ver}&encoding=json"),
        _ => format!("{base}/?v={ver}&encoding=json"),
    };
    tracing::info!("gateway: подключение к {url}");

    let (ws, _) = connect_async(url)
        .await
        .context("не удалось открыть WebSocket до gateway.discord.gg")?;
    let (mut writer, mut reader) = wsutil::split(ws);

    // ---- HELLO (op 10): the server dictates our heartbeat cadence ----
    let hello = wsutil::next_json(&mut reader).await?;
    if hello["op"].as_u64() != Some(10) {
        bail!("ожидали HELLO (op 10), получили op {:?}", hello["op"]);
    }
    let hb_interval = hello["d"]["heartbeat_interval"]
        .as_u64()
        .unwrap_or(41_250)
        .clamp(1000, 300_000);
    tracing::info!("gateway: HELLO, heartbeat каждые {hb_interval} мс");

    // ---- IDENTIFY (op 2) or RESUME (op 6) ----
    let token = state
        .token
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow!("нет токена"))?;
    if let (Some(id), Some(seq)) = (&session.id, session.seq) {
        tracing::info!("gateway: RESUME сессии {id} с seq={seq}");
        wsutil::send_json(
            &mut writer,
            &json!({
                "op": 6,
                "d": { "token": token, "session_id": id, "seq": seq }
            }),
        )
        .await?;
    } else {
        let is_bot = state.is_bot.load(Ordering::SeqCst);
        let build = state.build_number.load(Ordering::SeqCst);
        // Юзер-сессия: официальный клиент НЕ шлёт intents — вместо них
        // capabilities + client_state. С intents юзер-токен получает
        // урезанный поток событий (гильдии могут не приехать вовсе).
        // Бот-сессия: как раньше, интенты обязательны (v10).
        let d = if is_bot {
            tracing::info!(
                "gateway: IDENTIFY (бот) с intents={}",
                state.intents.load(Ordering::SeqCst)
            );
            json!({
                "token": token,
                "intents": state.intents.load(Ordering::SeqCst),
                "properties": client_properties(build),
                "presence": {
                    "status": "online",
                    "since": null,
                    "activities": [],
                    "afk": false
                },
                "compress": false
            })
        } else {
            tracing::info!(
                "gateway: IDENTIFY (юзер) capabilities={USER_CAPABILITIES} без интентов, presence=unknown"
            );
            json!({
                "token": token,
                "capabilities": USER_CAPABILITIES,
                "properties": client_properties(build),
                "presence": {
                    "status": "unknown",
                    "since": 0,
                    "activities": [],
                    "afk": false
                },
                "compress": false,
                "client_state": { "guild_versions": {} }
            })
        };
        wsutil::send_json(&mut writer, &json!({ "op": 2, "d": d })).await?;
    }

    let mut heartbeat = tokio::time::interval(Duration::from_millis(hb_interval));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await; // consume the immediate first tick

    let mut acked = true;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                let _ = writer.send(Message::Close(None)).await;
                return Ok(End::Shutdown);
            }

            cmd = rx.recv() => {
                match cmd {
                    Some(GatewayCommand::RequestMembers { guild_id }) => {
                        // op 8: query="" + limit=0 → the server streams every member in chunks
                        wsutil::send_json(&mut writer, &json!({
                            "op": 8,
                            "d": { "guild_id": guild_id, "query": "", "limit": 0 }
                        })).await?;
                    }
                    Some(GatewayCommand::BulkGuildSubscribe { guild_ids }) => {
                        // op 37 (юзер-сессии): подписка на события всех гильдий
                        // сразу — presences/typing/threads/member_updates.
                        let mut subs = serde_json::Map::new();
                        for gid in &guild_ids {
                            subs.insert(gid.clone(), json!({
                                "typing": true, "threads": true,
                                "activities": true, "member_updates": true
                            }));
                        }
                        wsutil::send_json(&mut writer, &json!({
                            "op": 37,
                            "d": { "subscriptions": subs }
                        })).await?;
                        tracing::info!("gateway: op 37 bulk subscribe на {} гильдий", guild_ids.len());
                    }
                    Some(GatewayCommand::SubscribeMemberList { guild_id, channels }) => {
                        // op 14 (юзер-сессии): ленивый список участников по
                        // диапазонам канала — приезжает GUILD_MEMBERS_CHUNK.
                        // threads: false — сверь с discord-client-rs
                        // (create_op_14: typing=true, threads=false,
                        // activities=true): байт-в-байт наш payload отличался
                        // от эталона ровно этим полем, и живой gateway отвечал
                        // на наш вариант close 4002 «Error while decoding
                        // payload» — цикл крашей при клике по серверу.
                        let mut chans = serde_json::Map::new();
                        for (cid, ranges) in &channels {
                            // ranges = [0, 99] → JSON [[0, 99]] на канал.
                            chans.insert(cid.clone(), json!([ranges]));
                        }
                        wsutil::send_json(&mut writer, &json!({
                            "op": 14,
                            "d": {
                                "guild_id": guild_id,
                                "typing": true,
                                "threads": false,
                                "activities": true,
                                "channels": chans
                            }
                        })).await?;
                        tracing::debug!("gateway: op 14 member list для {guild_id} ({} каналов)", channels.len());
                    }
                    Some(GatewayCommand::VoiceStateUpdate { guild_id, channel_id, self_mute, self_deaf, self_video }) => {
                        // op 4: войти/покинуть голосовой канал. Для звонков в
                        // ЛС/группе guild_id = null — так шлёт официальный
                        // клиент (voice-сервер идентифицирует по channel_id).
                        wsutil::send_json(
                            &mut writer,
                            &json!({
                                "op": 4,
                                "d": {
                                    "guild_id": guild_id,     // null → звонок в ЛС
                                    "channel_id": channel_id, // null → покинуть канал
                                    "self_mute": self_mute,
                                    "self_deaf": self_deaf,
                                    "self_video": self_video
                                }
                            }),
                        ).await?;
                    }
                    Some(GatewayCommand::StreamCreate { stream_type, guild_id, channel_id }) => {
                        // op 18 STREAM_CREATE — «показать экран»/«включить
                        // камеру». Сервер разошлёт dispatch STREAM_CREATE с
                        // stream_key — по нему останавливаем (op 20) и ставим
                        // на паузу (op 22).
                        wsutil::send_json(&mut writer, &json!({
                            "op": 18,
                            "d": {
                                "type": stream_type,
                                "guild_id": guild_id,
                                "channel_id": channel_id,
                                "preferred_region": null
                            }
                        })).await?;
                        tracing::info!("gateway: op 18 STREAM_CREATE отправлен");
                    }
                    Some(GatewayCommand::StreamDelete { stream_key }) => {
                        wsutil::send_json(&mut writer, &json!({
                            "op": 20,
                            "d": { "stream_key": stream_key }
                        })).await?;
                        tracing::info!("gateway: op 20 STREAM_DELETE {stream_key}");
                    }
                    Some(GatewayCommand::StreamSetPaused { stream_key, paused }) => {
                        wsutil::send_json(&mut writer, &json!({
                            "op": 22,
                            "d": { "stream_key": stream_key, "paused": paused }
                        })).await?;
                    }
                    Some(GatewayCommand::SendText(text)) => {
                        writer.send(Message::Text(text)).await?;
                    }
                    None => return Ok(End::Shutdown), // все sender'ы закрыты
                }
            }

            _ = heartbeat.tick() => {
                if !acked {
                    // Два пульса подряд без ACK → «зомби»-соединение.
                    tracing::warn!("gateway: нет HEARTBEAT_ACK — соединение протухло");
                    return Ok(End::Reconnect);
                }
                wsutil::send_json(&mut writer, &json!({ "op": 1, "d": session.seq })).await?;
                acked = false;
            }

            msg = reader.next() => {
                let Some(msg) = msg else {
                    tracing::warn!("gateway: поток закрылся");
                    return Ok(End::Reconnect);
                };
                let msg = msg.context("gateway ws ошибка")?;
                match msg {
                    Message::Text(text) => {
                        let v: Value = serde_json::from_str(&text).context("битый JSON кадра")?;
                        match v["op"].as_u64().unwrap_or(u64::MAX) {
                            0 => {
                                // DISPATCH: t = event name, s = sequence, d = payload.
                                // Разбор больше не фатален: кривое событие
                                // логируется внутри, но сессию НЕ убивает.
                                let t = v["t"].as_str().unwrap_or("").to_string();
                                if let Some(s) = v["s"].as_u64() {
                                    session.seq = Some(s);
                                }
                                let d = v["d"].clone();
                                handle_dispatch(app, state, &t, d, session).await;
                            }
                            1 => {
                                // Сервер сам попросил пульс — отвечаем немедленно.
                                wsutil::send_json(&mut writer, &json!({ "op": 1, "d": session.seq })).await?;
                            }
                            7 => {
                                tracing::info!("gateway: сервер попросил RECONNECT");
                                return Ok(End::Reconnect);
                            }
                            9 => {
                                let resumable = v["d"].as_bool().unwrap_or(false);
                                tracing::warn!("gateway: INVALID_SESSION (resumable={resumable})");
                                if !resumable {
                                    session.id = None;
                                    session.resume_url = None;
                                }
                                // документированная пауза 1–5 c перед повтором
                                let jitter = 1000 + (std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH).unwrap()
                                    .subsec_millis() % 4000);
                                tokio::time::sleep(Duration::from_millis(jitter as u64)).await;
                                return Ok(End::Reconnect);
                            }
                            11 => { acked = true; }
                            other => tracing::debug!("gateway: игнорируем op {other}"),
                        }
                    }
                    Message::Close(Some(cf)) => {
                        let code = u16::from(cf.code);
                        tracing::warn!("gateway: закрыто сервером, код {code} ({})", cf.reason);
                        match code {
                            4004 => return Ok(End::Fatal("Токен отклонён (close 4004). Возможно, сессия сброшена — войди заново.".into())),
                            4010 | 4011 => return Ok(End::Fatal(format!("Неподдерживаемая конфигурация шардирования (close {code})."))),
                            4013 | 4014 => {
                                // Privileged intents выключены в Developer Portal.
                                state.intents.store(INTENTS_FALLBACK, Ordering::SeqCst);
                                session.id = None;
                                tracing::warn!("gateway: интенты запрещены (4014) — деградируем на fallback-набор");
                                let _ = app.emit(
                                    "discord://warning",
                                    json!({ "message": "Privileged Intents выключены в Developer Portal — перезапускаюсь без Presence/Members/MessageContent. Включи их для полной функциональности." }),
                                );
                                return Ok(End::Reconnect);
                            }
                            4002 | 4005 => {
                                // Декод нашего исходящего payload сломан /
                                // повторный IDENTIFY. Сессия недееспособна:
                                // RESUME на неё всегда ответит INVALID_SESSION —
                                // сразу чистим и идём на свежий IDENTIFY.
                                session.id = None;
                                session.resume_url = None;
                                state.emit_status(
                                    "reconnecting",
                                    Some(format!("close {code}: {}", cf.reason)),
                                );
                                return Ok(End::Reconnect);
                            }
                            // Причина обрыва — в detail статуса: фронтенд показывает
                            // её в баннере, чтобы не гадать «почему отвалилось».
                            _ => {
                                state.emit_status(
                                    "reconnecting",
                                    Some(format!("close {code}: {}", cf.reason)),
                                );
                                return Ok(End::Reconnect);
                            }
                        }
                    }
                    Message::Close(None) => return Ok(End::Reconnect),
                    _ => {}
                }
            }
        }
    }
}
