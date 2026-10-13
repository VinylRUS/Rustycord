//! Tauri IPC commands — the seam the Svelte UI calls through `invoke()`.
//! Each command is a thin, typed wrapper over the REST client or gateway.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::State;

use crate::gateway::client::run_gateway;
use crate::rest::client::{urlencode, RestClient};
use crate::rest::models::{Channel, CurrentUser, GatewayBot, Message};
use crate::snowflake;
use crate::state::{AppState, GatewayCommand, INTENTS_FULL};
use crate::voice::coordinator;

fn es<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn rest_of(state: &State<'_, Arc<AppState>>) -> Result<RestClient, String> {
    let token = state
        .token
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| "Не подключено — войди через Bot Token".to_string())?;
    RestClient::new(&token).map_err(es)
}

/// Stop gateway + voice tasks (if running).
async fn stop_tasks(state: &State<'_, Arc<AppState>>) {
    if let Some(task) = state.gateway_task.lock().await.take() {
        task.abort();
    }
    state.shutdown.read().unwrap().cancel();
    *state.gateway_tx.write().unwrap() = None;
    let mut voices = state.voice_tasks.lock().await;
    for (_, (handle, token)) in voices.drain() {
        token.cancel();
        handle.abort();
    }
    state.voice_handshake.lock().unwrap().clear();
}

/// Validate the token, snapshot gateway info, spawn the gateway task.
#[tauri::command]
pub async fn connect(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    token: String,
) -> Result<CurrentUser, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Пустой токен".into());
    }

    stop_tasks(&state).await;

    // Fail fast on invalid token / network before spawning anything.
    let rest = RestClient::new(&token).map_err(es)?;
    let me: CurrentUser = rest.get_json("/users/@me").await.map_err(|e| {
        // 401 → понятное сообщение вместо сырой ошибки.
        let msg = e.to_string();
        if msg.contains("401") {
            "Неверный токен (401). Перепроверь его в Developer Portal → Bot → Reset Token."
                .to_string()
        } else {
            msg
        }
    })?;
    let gw: GatewayBot = rest.get_json("/gateway/bot").await.map_err(es)?;

    // Fresh state.
    state.cache.write().unwrap().reset();
    state.cache.write().unwrap().user = Some(me.clone());
    *state.token.write().unwrap() = Some(token);
    state.intents.store(INTENTS_FULL, Ordering::SeqCst);
    *state.gateway_url.write().unwrap() = gw.url;
    tracing::info!(
        "connect: бот @{} (id {}), shards={:?}",
        me.username,
        me.id,
        gw.shards
    );

    // Gateway task.
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    *state.gateway_tx.write().unwrap() = Some(tx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    *state.shutdown.write().unwrap() = shutdown.clone();

    let shared: Arc<AppState> = Arc::clone(state.inner());
    let task = tauri::async_runtime::spawn(run_gateway(
        app.clone(),
        shared.clone(),
        rx,
        shutdown.clone(),
    ));
    *state.gateway_task.lock().await = Some(task);

    // Voice coordinator (idempotent, dies with the same shutdown token).
    tauri::async_runtime::spawn(coordinator::coordinator(app.clone(), shared, shutdown));

    Ok(me)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    stop_tasks(&state).await;
    state.cache.write().unwrap().reset();
    *state.token.write().unwrap() = None;
    state.emit_status("shutdown", None);
    Ok(())
}

/// Full cache snapshot (used for hot-reload recovery).
#[tauri::command]
pub async fn get_state(state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let cache = state.cache.read().unwrap();
    let guilds: Vec<Value> = cache
        .guilds
        .values()
        .map(|g| {
            json!({
                "id": g.id,
                "name": g.name,
                "icon": g.icon,
                "banner": g.banner,
                "owner_id": g.owner_id,
                "roles": g.roles,
                "channels": g.channels,
                "members": g.members.values().collect::<Vec<_>>(),
                "presences": g.presences.values().collect::<Vec<_>>(),
                "voice_states": g.voice_states,
                "member_count": g.member_count,
            })
        })
        .collect();
    Ok(json!({
        "user": cache.user,
        "guilds": guilds,
        "dms": cache.dms,
    }))
}

#[tauri::command]
pub async fn get_dm_channels(state: State<'_, Arc<AppState>>) -> Result<Vec<Channel>, String> {
    let rest = rest_of(&state)?;
    let dms: Vec<Channel> = rest.get_json("/users/@me/channels").await.map_err(es)?;
    {
        let mut cache = state.cache.write().unwrap();
        for c in &dms {
            if !cache.dms.iter().any(|x| x.id == c.id) {
                cache.dms.push(c.clone());
            }
        }
    }
    Ok(dms)
}

#[tauri::command]
pub async fn get_messages(
    state: State<'_, Arc<AppState>>,
    channel_id: String,
    limit: Option<u32>,
    before: Option<String>,
) -> Result<Vec<Message>, String> {
    let rest = rest_of(&state)?;
    let mut query: Vec<(String, String)> = vec![(
        "limit".into(),
        limit.unwrap_or(50).clamp(1, 100).to_string(),
    )];
    if let Some(b) = before {
        query.push(("before".into(), b));
    }
    let path = format!("/channels/{channel_id}/messages");
    let list: Vec<Message> = rest.get_json_q(&path, &query).await.map_err(es)?;
    Ok(list)
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, Arc<AppState>>,
    channel_id: String,
    content: String,
) -> Result<Message, String> {
    let rest = rest_of(&state)?;
    let path = format!("/channels/{channel_id}/messages");
    let msg: Message = rest
        .post_json(&path, json!({ "content": content, "tts": false }))
        .await
        .map_err(es)?;
    Ok(msg)
}

#[tauri::command]
pub async fn edit_message(
    state: State<'_, Arc<AppState>>,
    channel_id: String,
    message_id: String,
    content: String,
) -> Result<Message, String> {
    let rest = rest_of(&state)?;
    let path = format!("/channels/{channel_id}/messages/{message_id}");
    let msg: Message = rest
        .patch_json(&path, json!({ "content": content }))
        .await
        .map_err(es)?;
    Ok(msg)
}

#[tauri::command]
pub async fn delete_message(
    state: State<'_, Arc<AppState>>,
    channel_id: String,
    message_id: String,
) -> Result<(), String> {
    let rest = rest_of(&state)?;
    let path = format!("/channels/{channel_id}/messages/{message_id}");
    rest.delete::<()>(&path).await.map_err(es)?;
    Ok(())
}

#[tauri::command]
pub async fn add_reaction(
    state: State<'_, Arc<AppState>>,
    channel_id: String,
    message_id: String,
    emoji: String,
) -> Result<(), String> {
    let rest = rest_of(&state)?;
    let path = format!(
        "/channels/{channel_id}/messages/{message_id}/reactions/{}/@me",
        urlencode(&emoji)
    );
    rest.put_empty(&path).await.map_err(es)?;
    Ok(())
}

#[tauri::command]
pub async fn trigger_typing(
    state: State<'_, Arc<AppState>>,
    channel_id: String,
) -> Result<(), String> {
    let rest = rest_of(&state)?;
    let path = format!("/channels/{channel_id}/typing");
    rest.post_empty(&path).await.map_err(es)?;
    Ok(())
}

/// op 8 — ask the gateway for the full member list (arrives in chunks).
#[tauri::command]
pub async fn request_members(
    state: State<'_, Arc<AppState>>,
    guild_id: String,
) -> Result<(), String> {
    let tx = state
        .gateway_tx
        .read()
        .unwrap()
        .clone()
        .ok_or("Gateway не подключён")?;
    tx.send(GatewayCommand::RequestMembers { guild_id })
        .await
        .map_err(es)?;
    Ok(())
}

/// Escape hatch: push a raw JSON frame onto the gateway websocket.
/// Instrument for protocol experiments — e.g. `{"op": 8, "d": {...}}`.
#[tauri::command]
pub async fn gateway_send_raw(
    state: State<'_, Arc<AppState>>,
    frame: String,
) -> Result<(), String> {
    // Fail fast on non-JSON so typos don't kill the websocket.
    serde_json::from_str::<Value>(&frame).map_err(|e| format!("Невалидный JSON: {e}"))?;
    let tx = state
        .gateway_tx
        .read()
        .unwrap()
        .clone()
        .ok_or("Gateway не подключён")?;
    tx.send(GatewayCommand::SendText(frame)).await.map_err(es)?;
    Ok(())
}

#[tauri::command]
pub async fn kick_member(
    state: State<'_, Arc<AppState>>,
    guild_id: String,
    user_id: String,
) -> Result<(), String> {
    let rest = rest_of(&state)?;
    let path = format!("/guilds/{guild_id}/members/{user_id}");
    rest.delete::<()>(&path).await.map_err(|e| {
        let m = e.to_string();
        if m.contains("403") {
            "Нет прав: нужны KICK_MEMBERS и роль выше цели".to_string()
        } else {
            m
        }
    })?;
    Ok(())
}

#[tauri::command]
pub async fn ban_member(
    state: State<'_, Arc<AppState>>,
    guild_id: String,
    user_id: String,
) -> Result<(), String> {
    let rest = rest_of(&state)?;
    let path = format!("/guilds/{guild_id}/bans/{user_id}");
    rest.put_empty(&path).await.map_err(|e| {
        let m = e.to_string();
        if m.contains("403") {
            "Нет прав: нужны BAN_MEMBERS и роль выше цели".to_string()
        } else {
            m
        }
    })?;
    Ok(())
}

/// op 4 — join/leave a voice channel (channel_id = None → leave),
/// self mute/deafen flags ride along the same frame.
#[tauri::command]
pub async fn join_voice(
    state: State<'_, Arc<AppState>>,
    guild_id: String,
    channel_id: Option<String>,
    self_mute: bool,
    self_deaf: bool,
) -> Result<(), String> {
    let tx = state
        .gateway_tx
        .read()
        .unwrap()
        .clone()
        .ok_or("Gateway не подключён")?;
    tx.send(GatewayCommand::VoiceStateUpdate {
        guild_id,
        channel_id,
        self_mute,
        self_deaf,
    })
    .await
    .map_err(es)?;
    Ok(())
}

#[tauri::command]
pub async fn parse_snowflake(id: String) -> Result<snowflake::SnowflakeInfo, String> {
    snowflake::parse(&id).map_err(es)
}
