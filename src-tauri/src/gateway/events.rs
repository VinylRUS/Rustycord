//! Gateway DISPATCH routing: update the Rust cache, forward everything to
//! the UI, and assemble voice handshakes from the two-voice-event dance.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tauri::Emitter;

use crate::rest::models::{Channel, Guild, Member, Presence, VoiceState};
use crate::state::{GuildState, SharedState};
use crate::voice::VoiceControls;

/// Session state accumulated from DISPATCH events. Lives across reconnects
/// so the gateway loop can RESUME instead of re-IDENTIFYing.
#[derive(Default)]
pub struct SessionData {
    /// `session_id` from READY — until it arrives there is nothing to resume.
    pub id: Option<String>,
    /// `resume_gateway_url` from READY — dedicated websocket URL for RESUME.
    pub resume_url: Option<String>,
    /// Last `s` (sequence number) seen on a DISPATCH; sent with every
    /// heartbeat and replayed in RESUME so Discord can fill the gap.
    pub seq: Option<u64>,
}

pub async fn handle_dispatch(
    app: &tauri::AppHandle,
    state: &SharedState,
    t: &str,
    d: Value,
    session: &mut SessionData,
) -> Result<()> {
    match t {
        "READY" => {
            let user_id = d["user"]["id"].as_str().unwrap_or_default().to_string();
            let username = d["user"]["username"].as_str().unwrap_or("?").to_string();
            let session_id = d["session_id"].as_str().unwrap_or_default().to_string();
            session.id = Some(session_id.clone());
            session.resume_url = d["resume_gateway_url"].as_str().map(String::from);
            if let Some(user) = d.get("user").cloned() {
                if let Ok(u) = serde_json::from_value(user.clone()) {
                    state.cache.write().unwrap().user = Some(u);
                }
            }
            // Пользовательский READY (в отличие от бот-READY) несёт DM-каналы
            // прямо в private_channels — кладём их в кэш, чтобы UI сразу
            // видел список личных сообщений.
            if let Some(dms) = d.get("private_channels").and_then(|v| v.as_array()) {
                let mut cache = state.cache.write().unwrap();
                for ch in dms {
                    if let Ok(c) = serde_json::from_value::<Channel>(ch.clone()) {
                        if !cache.dms.iter().any(|x| x.id == c.id) {
                            cache.dms.push(c);
                        }
                    }
                }
            }
            tracing::info!("gateway: READY — {username} ({user_id}), session {session_id}");
            let _ = app.emit(
                "discord://ready",
                json!({ "user": d["user"], "session_id": session_id }),
            );
            state.emit_status("connected", None);
        }

        "GUILD_CREATE" => {
            let g: Guild =
                serde_json::from_value(d.clone()).context("не удалось разобрать GUILD_CREATE")?;
            let needs_chunk = g.members.as_ref().map_or(true, |m| m.is_empty());
            {
                let mut cache = state.cache.write().unwrap();
                let mut gs = GuildState {
                    id: g.id.clone(),
                    name: g.name.clone(),
                    icon: g.icon.clone(),
                    banner: g.banner.clone(),
                    owner_id: g.owner_id.clone(),
                    roles: g.roles.clone(),
                    channels: Vec::new(),
                    members: Default::default(),
                    presences: Default::default(),
                    voice_states: Vec::new(),
                    member_count: g.member_count.unwrap_or(0),
                };
                if let Some(chs) = &g.channels {
                    gs.channels = chs.clone();
                }
                if let Some(members) = &g.members {
                    for m in members {
                        if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                            gs.members.insert(uid, m.clone());
                        }
                    }
                }
                if let Some(presences) = &g.presences {
                    for p in presences {
                        gs.presences.insert(p.user.id.clone(), p.clone());
                    }
                }
                if let Some(vs) = &g.voice_states {
                    gs.voice_states = vs.clone();
                }
                cache.guilds.insert(g.id.clone(), gs);
            }
            tracing::info!(
                "gateway: GUILD_CREATE «{}» ({} каналов)",
                g.name,
                g.channels.as_ref().map(Vec::len).unwrap_or(0)
            );
            // Большие гильдии присылают участников пачками по запросу (op 8).
            if needs_chunk {
                if let Some(tx) = state.gateway_tx.read().unwrap().as_ref() {
                    let _ = tx.try_send(crate::state::GatewayCommand::RequestMembers {
                        guild_id: g.id.clone(),
                    });
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBERS_CHUNK" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let members: Vec<Member> =
                serde_json::from_value(d["members"].clone()).unwrap_or_default();
            let chunk_index = d["chunk_index"].as_u64().unwrap_or(0);
            let chunk_count = d["chunk_count"].as_u64().unwrap_or(1);
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    for m in members {
                        if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                            gs.members.insert(uid, m);
                        }
                    }
                }
            }
            tracing::debug!(
                "gateway: чанк участников {}/{} для гильдии {}",
                chunk_index + 1,
                chunk_count,
                guild_id
            );
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBER_ADD" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Ok(m) = serde_json::from_value::<Member>(d["member"].clone()) {
                if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                        gs.members.insert(uid, m);
                        gs.member_count += 1;
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBER_UPDATE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Ok(m) = serde_json::from_value::<Member>(d.clone()) {
                if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                        gs.members.insert(uid, m);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBER_REMOVE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let uid = d["user"]["id"].as_str().unwrap_or_default().to_string();
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    gs.members.remove(&uid);
                    gs.member_count = gs.member_count.saturating_sub(1);
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_UPDATE" => {
            if let Ok(g) = serde_json::from_value::<Guild>(d.clone()) {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&g.id) {
                    gs.name = g.name;
                    gs.icon = g.icon;
                    gs.banner = g.banner;
                    gs.owner_id = g.owner_id;
                    gs.roles = g.roles;
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_DELETE" => {
            let gid = d["id"].as_str().unwrap_or_default().to_string();
            state.cache.write().unwrap().guilds.remove(&gid);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Ok(role) = serde_json::from_value::<crate::rest::models::Role>(d["role"].clone())
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    gs.roles.retain(|r| r.id != role.id);
                    gs.roles.push(role);
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_ROLE_DELETE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let role_id = d["role_id"].as_str().unwrap_or_default().to_string();
            let mut cache = state.cache.write().unwrap();
            if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                gs.roles.retain(|r| r.id != role_id);
            }
            drop(cache);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "CHANNEL_CREATE" => {
            if let Ok(c) = serde_json::from_value::<Channel>(d.clone()) {
                let is_dm = c.guild_id.is_none() && (c.kind == 1 || c.kind == 3);
                if is_dm {
                    let mut cache = state.cache.write().unwrap();
                    if !cache.dms.iter().any(|x| x.id == c.id) {
                        cache.dms.push(c);
                    }
                } else if let Some(gid) = &c.guild_id {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(gid) {
                        if !gs.channels.iter().any(|x| x.id == c.id) {
                            gs.channels.push(c);
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "CHANNEL_UPDATE" => {
            if let Ok(c) = serde_json::from_value::<Channel>(d.clone()) {
                if let Some(gid) = &c.guild_id {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(gid) {
                        if let Some(slot) = gs.channels.iter_mut().find(|x| x.id == c.id) {
                            *slot = c;
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "CHANNEL_DELETE" => {
            if let Ok(c) = serde_json::from_value::<Channel>(d.clone()) {
                let mut cache = state.cache.write().unwrap();
                cache.dms.retain(|x| x.id != c.id);
                if let Some(gid) = &c.guild_id {
                    if let Some(gs) = cache.guilds.get_mut(gid) {
                        gs.channels.retain(|x| x.id != c.id);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "PRESENCE_UPDATE" => {
            if let Ok(p) = serde_json::from_value::<Presence>(d.clone()) {
                if let Some(gid) = p.guild_id.clone() {
                    let uid = p.user.id.clone();
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&gid) {
                        gs.presences.insert(uid, p);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "VOICE_STATE_UPDATE" => {
            if let Ok(vs) = serde_json::from_value::<VoiceState>(d.clone()) {
                let Some(gid) = vs.guild_id.clone() else {
                    return Ok(());
                };
                {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&gid) {
                        gs.voice_states.retain(|v| v.user_id != vs.user_id);
                        if vs.channel_id.is_some() {
                            gs.voice_states.push(vs.clone());
                        }
                    }
                }
                // Голосовой рукопожатие для нас самих: запоминаем session_id.
                let is_self = state.self_id().as_deref() == Some(vs.user_id.as_str());
                if is_self {
                    if let (Some(sid), true) = (vs.session_id.clone(), vs.channel_id.is_some()) {
                        let mut hs = state.voice_handshake.lock().unwrap();
                        let entry = hs.entry(gid.clone()).or_default();
                        entry.guild_id = gid.clone();
                        entry.user_id = vs.user_id.clone();
                        entry.session_id = Some(sid);
                        drop(hs);
                        // Эхо op4: подтягиваем живую сессию к актуальным флагам.
                        let sessions = state.voice_sessions.lock().unwrap();
                        if let Some(handle) = sessions.get(&gid) {
                            *handle.controls.lock().unwrap() =
                                VoiceControls::new(vs.self_mute, vs.self_deaf);
                        }
                    } else if vs.channel_id.is_none() {
                        // Выход из войса: гасим голосовую сессию.
                        state.voice_handshake.lock().unwrap().remove(&gid);
                        state.voice_sessions.lock().unwrap().remove(&gid);
                        state.voice_desired.lock().unwrap().remove(&gid);
                        let mut tasks = state.voice_tasks.lock().await;
                        if let Some((handle, token)) = tasks.remove(&gid) {
                            token.cancel();
                            handle.abort();
                        }
                        let _ = app.emit(
                            "discord://voice",
                            json!({ "kind": "disconnected", "data": { "guild_id": gid } }),
                        );
                    }
                }
                state.voice_notify.notify_one();
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "VOICE_SERVER_UPDATE" => {
            let gid = d["guild_id"].as_str().unwrap_or_default().to_string();
            let token = d["token"].as_str().unwrap_or_default().to_string();
            let endpoint = d["endpoint"].as_str().unwrap_or_default().to_string();
            if !gid.is_empty() && !token.is_empty() {
                let user_id = state.self_id().unwrap_or_default();
                let mut hs = state.voice_handshake.lock().unwrap();
                let entry = hs.entry(gid.clone()).or_default();
                entry.guild_id = gid.clone();
                entry.user_id = user_id;
                entry.token = Some(token);
                entry.endpoint = Some(endpoint);
            }
            state.voice_notify.notify_one();
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        // Сообщения, реакции, typing и всё остальное идут в UI напрямую —
        // кэш сообщений живёт на стороне фронтенда (как IndexedDB у оригинала).
        _ => {
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }
    }
    Ok(())
}
