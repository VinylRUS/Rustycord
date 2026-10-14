//! Gateway DISPATCH routing: update the Rust cache, forward everything to
//! the UI, and assemble voice handshakes from the two-voice-event dance.

use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

use crate::rest::models::{Channel, Guild, Member, Presence, Relationship, VoiceState};
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
) {
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
            let mut stub_ids: Vec<String> = Vec::new();
            {
                let mut cache = state.cache.write().unwrap();
                // Пользовательский READY (в отличие от бот-READY) несёт DM-каналы
                // прямо в private_channels — кладём их в кэш, чтобы UI сразу
                // видел список личных сообщений.
                if let Some(dms) = d.get("private_channels").and_then(|v| v.as_array()) {
                    for ch in dms {
                        if let Ok(c) = serde_json::from_value::<Channel>(ch.clone()) {
                            if !cache.dms.iter().any(|x| x.id == c.id) {
                                cache.dms.push(c);
                            }
                        }
                    }
                }
                // Друзья/заявки/блок-лист — тоже часть пользовательского READY.
                // Бот-аккаунты этот массив не получают.
                if let Some(rels) = d.get("relationships").and_then(|v| v.as_array()) {
                    for r in rels {
                        if let Ok(rel) = serde_json::from_value::<Relationship>(r.clone()) {
                            if let Some(uid) = r
                                .get("user")
                                .and_then(|u| u.get("id"))
                                .and_then(|v| v.as_str())
                            {
                                cache.relationships.insert(uid.to_string(), rel);
                            }
                        }
                    }
                }
                // READY.presences у юзер-сессии — это статусы друзей (без
                // guild_id); разкладываем их отдельно от гильдейских.
                if let Some(presences) = d.get("presences").and_then(|v| v.as_array()) {
                    for p in presences {
                        if let Ok(pr) = serde_json::from_value::<Presence>(p.clone()) {
                            if pr.guild_id.is_none() {
                                cache.friend_presences.insert(pr.user.id.clone(), pr);
                            }
                        }
                    }
                }
                // Гильдии-стабы из READY: юзер-READY несёт их с id/name/icon —
                // этого хватает, чтобы нарисовать рейл серверов СРАЗУ,
                // не дожидаясь GUILD_CREATE (которые приезжают только после
                // подписки op 37). Полные данные подтянутся GUILD_CREATE'ом.
                if let Some(guilds) = d.get("guilds").and_then(|v| v.as_array()) {
                    for g in guilds {
                        let Some(id) = g.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        stub_ids.push(id.to_string());
                        if cache.guilds.contains_key(id) {
                            continue; // RESUME: полная гильдия уже в кэше
                        }
                        cache.guilds.insert(
                            id.to_string(),
                            GuildState {
                                id: id.to_string(),
                                name: g
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("…")
                                    .to_string(),
                                icon: g.get("icon").and_then(|v| v.as_str()).map(String::from),
                                banner: None,
                                owner_id: g
                                    .get("owner_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                roles: Vec::new(),
                                emojis: Vec::new(),
                                channels: Vec::new(),
                                members: Default::default(),
                                presences: Default::default(),
                                voice_states: Vec::new(),
                                member_count: 0,
                            },
                        );
                    }
                }
                // merged_members[i] / merged_presences параллельны d.guilds:
                // собственная карточка участника и статусы друзей по гильдиям.
                // ВАЖНО: вне зоны cache.write() — apply_merged берёт лок сам.
            }
            apply_merged(state, &d);
            let rel_count = d
                .get("relationships")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let dm_count = d
                .get("private_channels")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            tracing::info!(
                "gateway: READY — {username} ({user_id}), session {session_id}, {rel_count} отношений, {dm_count} ЛС"
            );
            let _ = app.emit(
                "discord://ready",
                json!({ "user": d["user"], "session_id": session_id }),
            );
            // ГЛАВНЫЙ ФИКС видимости серверов: юзер-сессии получают
            // GUILD_CREATE только на подписанные гильдии (op 37). Шторм
            // одним пакетом подозревался в обрывах — поэтому батчами по 50
            // с паузой 350 мс (мягче, чем один мегапакет и чем залп op 14
            // по всем гильдиям сразу, который валил сессию в v0.5.0).
            if !state.is_bot.load(Ordering::SeqCst) && !stub_ids.is_empty() {
                let tx = state.gateway_tx.read().unwrap().clone();
                if let Some(tx) = tx {
                    let st = Arc::clone(state);
                    let batches: Vec<Vec<String>> =
                        stub_ids.chunks(50).map(|c| c.to_vec()).collect();
                    tauri::async_runtime::spawn(async move {
                        for batch in batches {
                            if st.shutdown.read().unwrap().is_cancelled() {
                                break;
                            }
                            if tx
                                .send(crate::state::GatewayCommand::BulkGuildSubscribe {
                                    guild_ids: batch.clone(),
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                            tracing::info!(
                                "gateway: op 37 bulk subscribe — {} гильдий",
                                batch.len()
                            );
                            tokio::time::sleep(Duration::from_millis(350)).await;
                        }
                    });
                }
            }
            // Полный READY вперёд как dispatch: фронтенду нужны
            // private_channels (recipient_ids!), users, relationships
            // и presences, которых нет в коротком ready-событии выше.
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
            state.emit_status("connected", None);
        }

        "READY_SUPPLEMENTAL" => {
            // Приезжает сразу после потока GUILD_CREATE: голосовые состояния
            // гильдий, merged_members (своя карточка) и merged_presences
            // (статусы друзей и «видимые» участники по гильдиям).
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(guilds) = d.get("guilds").and_then(|v| v.as_array()) {
                    for g in guilds {
                        let Some(id) = g.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        let Some(gs) = cache.guilds.get_mut(id) else {
                            continue;
                        };
                        if let Some(vs) = g.get("voice_states") {
                            if let Ok(list) = serde_json::from_value::<Vec<VoiceState>>(vs.clone())
                            {
                                gs.voice_states = list;
                            }
                        }
                    }
                }
            }
            apply_merged(state, &d);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "RELATIONSHIP_ADD" => {
            // {id, type, user, nickname?} — id совпадает с user.id.
            if let Ok(rel) = serde_json::from_value::<Relationship>(d.clone()) {
                let uid = rel.user.id.clone();
                let kind = rel.kind;
                state
                    .cache
                    .write()
                    .unwrap()
                    .relationships
                    .insert(uid.clone(), rel);
                tracing::debug!("gateway: RELATIONSHIP_ADD {uid} (type {kind})");
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "RELATIONSHIP_REMOVE" => {
            let uid = d["id"].as_str().unwrap_or_default().to_string();
            if !uid.is_empty() {
                let mut cache = state.cache.write().unwrap();
                cache.relationships.remove(&uid);
                cache.friend_presences.remove(&uid);
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_CREATE" => {
            // Мягкий разбор: одна кривая гильдия не должна ронять ВСЮ сессию
            // (раньше `?` здесь убивал соединение → бесконечный reconnect).
            let parsed: Option<Guild> = serde_json::from_value(d.clone()).ok();
            let needs_chunk = parsed
                .as_ref()
                .map_or(true, |g| g.members.as_ref().map_or(true, |m| m.is_empty()));
            if let Some(g) = parsed {
                {
                    let mut cache = state.cache.write().unwrap();
                    let mut gs = GuildState {
                        id: g.id.clone(),
                        name: g.name.clone(),
                        icon: g.icon.clone(),
                        banner: g.banner.clone(),
                        owner_id: g.owner_id.clone(),
                        roles: g.roles.clone(),
                        emojis: g.emojis.clone().unwrap_or_default(),
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
                // op 14-подписка на список участников — ТОЛЬКО по запросу
                // (юзер-сессии: commands::subscribe_guild при открытии сервера).
                // Ботам оставлен привычный op 8 на полную выгрузку участников.
                if needs_chunk && state.is_bot.load(Ordering::SeqCst) {
                    if let Some(tx) = state.gateway_tx.read().unwrap().as_ref() {
                        let _ = tx.try_send(crate::state::GatewayCommand::RequestMembers {
                            guild_id: g.id.clone(),
                        });
                    }
                }
            } else {
                tracing::warn!(
                    "gateway: GUILD_CREATE не разобран (shape) — прокидываем сырое событие в UI"
                );
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBERS_CHUNK" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let members: Vec<Member> =
                serde_json::from_value(d["members"].clone()).unwrap_or_default();
            let chunk_index = d["chunk_index"].as_u64().unwrap_or(0);
            let chunk_count = d["chunk_count"].as_u64().unwrap_or(1);
            // Ответ на op 14 (activities: true) несёт и пресенсы — без них
            // список участников выглядит «все офлайн».
            let presences: Vec<Presence> =
                serde_json::from_value(d["presences"].clone()).unwrap_or_default();
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    for m in members {
                        if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                            gs.members.insert(uid, m);
                        }
                    }
                    for p in presences {
                        gs.presences.insert(p.user.id.clone(), p);
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
                    gs.emojis = g.emojis.unwrap_or_default();
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
                match p.guild_id.clone() {
                    Some(gid) => {
                        let uid = p.user.id.clone();
                        let mut cache = state.cache.write().unwrap();
                        if let Some(gs) = cache.guilds.get_mut(&gid) {
                            gs.presences.insert(uid, p);
                        }
                    }
                    None => {
                        // Дружеский статус (вне гильдий) — обновляем кэш друзей.
                        let uid = p.user.id.clone();
                        state.cache.write().unwrap().friend_presences.insert(uid, p);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "VOICE_STATE_UPDATE" => {
            if let Ok(vs) = serde_json::from_value::<VoiceState>(d.clone()) {
                let Some(gid) = vs.guild_id.clone() else {
                    return;
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
                        entry.channel_id = vs.channel_id.clone();
                        drop(hs);
                        // Эхо op4: подтягиваем живую сессию к актуальным флагам.
                        let sessions = state.voice_sessions.lock().unwrap();
                        if let Some(handle) = sessions.get(&gid) {
                            *handle.controls.lock().unwrap() =
                                VoiceControls::new(vs.self_mute, vs.self_deaf);
                            // Мьют в songbird: движок шлёт тишину вместо микрофона
                            // (деафиен тоже глушит передачу — как в оригинале).
                            // Driver — клон канала управления, mute уходит в задачи.
                            if let Some(mut driver) = handle.driver.clone() {
                                driver.mute(vs.self_mute || vs.self_deaf);
                            }
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
}

/// Разбор merged-массивов READY / READY_SUPPLEMENTAL: `merged_members[i]` и
/// `merged_presences.guilds[i]` параллельны `d.guilds[i]`, а
/// `merged_presences.friends` — статусы друзей (лечит «все не в сети»).
/// Вызывается ВНЕ любых блокировок кэша.
fn apply_merged(state: &SharedState, d: &Value) {
    let Some(guilds) = d.get("guilds").and_then(|v| v.as_array()) else {
        return;
    };
    let guild_id_at = |i: usize| -> Option<String> {
        guilds
            .get(i)
            .and_then(|g| g.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
    };
    let mut friend_n = 0usize;
    let mut member_n = 0usize;
    let mut guild_pres_n = 0usize;
    {
        let mut cache = state.cache.write().unwrap();
        // merged_presences.friends → статусы друзей.
        if let Some(friends) = d
            .get("merged_presences")
            .and_then(|mp| mp.get("friends"))
            .and_then(|v| v.as_array())
        {
            for p in friends {
                if let Ok(pr) = serde_json::from_value::<Presence>(p.clone()) {
                    cache.friend_presences.insert(pr.user.id.clone(), pr);
                    friend_n += 1;
                }
            }
        }
        // merged_presences.guilds[i] ↔ guilds[i] — статусы по гильдиям.
        if let Some(gp) = d
            .get("merged_presences")
            .and_then(|mp| mp.get("guilds"))
            .and_then(|v| v.as_array())
        {
            for (i, arr) in gp.iter().enumerate() {
                let Some(gid) = guild_id_at(i) else { continue };
                let Some(gs) = cache.guilds.get_mut(&gid) else {
                    continue;
                };
                for p in arr.as_array().into_iter().flatten() {
                    if let Ok(pr) = serde_json::from_value::<Presence>(p.clone()) {
                        gs.presences.insert(pr.user.id.clone(), pr);
                        guild_pres_n += 1;
                    }
                }
            }
        }
        // merged_members[i] ↔ guilds[i] — своя карточка участника (роли, ник).
        if let Some(mm) = d.get("merged_members").and_then(|v| v.as_array()) {
            for (i, arr) in mm.iter().enumerate() {
                let Some(gid) = guild_id_at(i) else { continue };
                let Some(gs) = cache.guilds.get_mut(&gid) else {
                    continue;
                };
                for m in arr.as_array().into_iter().flatten() {
                    if let Ok(mem) = serde_json::from_value::<Member>(m.clone()) {
                        if let Some(uid) = mem.user.as_ref().map(|u| u.id.clone()) {
                            gs.members.insert(uid, mem);
                            member_n += 1;
                        }
                    }
                }
            }
        }
    }
    if friend_n + member_n + guild_pres_n > 0 {
        tracing::info!(
            "gateway: merged-данные: {friend_n} статусов друзей, {guild_pres_n} гильдейских, {member_n} карточек участников"
        );
    }
}
