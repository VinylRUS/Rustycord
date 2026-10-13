//! Tauri IPC commands — the seam the Svelte UI calls through `invoke()`.
//! Each command is a thin, typed wrapper over the REST client or gateway.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::auth;
use crate::gateway::client::run_gateway;
use crate::rest::client::{urlencode, RestClient};
use crate::rest::models::{Channel, CurrentUser, Message, Relationship};
use crate::snowflake;
use crate::state::{AppState, GatewayCommand, INTENTS_USER};
use crate::superprops;
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
        .ok_or_else(|| "Не подключено — войди в аккаунт".to_string())?;
    let build = state.build_number.load(Ordering::SeqCst);
    RestClient::new(&token, build).map_err(es)
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

/* ============================ AUTH ============================ */

/// Шаг 1 входа по email+паролю. Возможен второй шаг (2FA) — тогда
/// возвращаем {status:"mfa", ticket} и UI просит код.
#[tauri::command]
pub async fn auth_login_email(email: String, password: String) -> Result<Value, String> {
    let email = email.trim().to_string();
    if email.is_empty() || password.is_empty() {
        return Err("Заполни email и пароль".into());
    }
    match auth::login::login(&email, &password).await {
        Ok(auth::login::LoginOutcome::Token(token)) => {
            Ok(json!({ "status": "ok", "token": token }))
        }
        Ok(auth::login::LoginOutcome::Mfa { ticket, sms }) => {
            Ok(json!({ "status": "mfa", "ticket": ticket, "sms": sms }))
        }
        Err(e) => Err(format!("{e:#}")),
    }
}

/// Шаг 2: код из приложения-аутентификатора (TOTP).
#[tauri::command]
pub async fn auth_submit_mfa(code: String, ticket: String) -> Result<Value, String> {
    match auth::login::submit_mfa(code.trim(), &ticket).await {
        Ok(auth::login::LoginOutcome::Token(token)) => {
            Ok(json!({ "status": "ok", "token": token }))
        }
        Ok(_) => Err("Неожиданный ответ 2FA".into()),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// QR-логин: стартуем remote-auth сессию. Дальше UI слушает события
/// `discord://qr` (fingerprint → scanned → done) и рисует QR-код.
#[tauri::command]
pub async fn auth_start_qr(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    qr_stop(&state).await;
    let cancel = CancellationToken::new();
    *state.qr_cancel.write().unwrap() = Some(cancel.clone());
    let app_for_task = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(e) = auth::remote::run_remote_auth(app_for_task.clone(), cancel).await {
            tracing::warn!("remote-auth: {e:#}");
            // Ошибка обязана попасть в UI — иначе QR просто «не появится».
            let _ = app_for_task.emit(
                "discord://qr",
                json!({ "stage": "error", "message": format!("{e:#}") }),
            );
        }
    });
    *state.qr_task.lock().await = Some(task);
    Ok(())
}

/// Отменить QR-сессию (пользователь ушёл с таба/закрыл экран логина).
#[tauri::command]
pub async fn auth_cancel_qr(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    qr_stop(&state).await;
    Ok(())
}

async fn qr_stop(state: &State<'_, Arc<AppState>>) {
    if let Some(cancel) = state.qr_cancel.write().unwrap().take() {
        cancel.cancel();
    }
    // Даём задаче ~мгновение на вежливый close-кадр перед abort.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    if let Some(task) = state.qr_task.lock().await.take() {
        task.abort();
    }
}

/* ========================== CONNECT =========================== */

/// Validate the user token, spawn the gateway + voice tasks.
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
    let build = state.build_number.load(Ordering::SeqCst);
    let rest = RestClient::new(&token, build).map_err(es)?;
    let me: CurrentUser = rest.get_json("/users/@me").await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("401") {
            "Токен отклонён (401) — сессия могла быть сброшена. Войди заново через QR или email."
                .to_string()
        } else {
            msg
        }
    })?;

    // Свежий client_build_number в фоне — следующие REST-вызовы возьмут его
    // из state (см. rest_of). /users/@me работает и со старым, так что
    // логин не ждёт.
    {
        let state_inner: Arc<AppState> = Arc::clone(state.inner());
        tauri::async_runtime::spawn(async move {
            if let Some(b) = superprops::fetch_build_number().await {
                state_inner.build_number.store(b, Ordering::SeqCst);
            }
        });
    }

    // Fresh state.
    state.cache.write().unwrap().reset();
    state.cache.write().unwrap().user = Some(me.clone());
    *state.token.write().unwrap() = Some(token);
    state.intents.store(INTENTS_USER, Ordering::SeqCst);
    // Пользовательская сессия ходит на общий gateway (без /gateway/bot,
    // это бот-эндпоинт — юзеру он вернёт 401).
    *state.gateway_url.write().unwrap() = "wss://gateway.discord.gg".to_string();
    tracing::info!("connect: пользователь @{} (id {})", me.username, me.id);

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

/* ===================== GENERIC REST (UI + plugins) ===================== */

/// Универсальный REST-прокси: любой метод/путь относительно /api/v10.
/// Это «escape hatch» для управления сервером и для плагинов — UI строит
/// запросы сам, Rust отвечает за заголовки и rate limits.
#[tauri::command]
pub async fn api_request(
    state: State<'_, Arc<AppState>>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    let rest = rest_of(&state)?;
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    let method = method.to_uppercase();
    let result: Result<Value, _> = match method.as_str() {
        "GET" => rest.get_json::<Value>(&path).await,
        "POST" => {
            rest.post_json::<Value>(&path, body.unwrap_or(Value::Null))
                .await
        }
        "PATCH" => {
            rest.patch_json::<Value>(&path, body.unwrap_or(Value::Null))
                .await
        }
        "PUT" => match body {
            Some(b) => rest.put_json::<Value>(&path, b).await,
            None => rest.put_empty(&path).await.map(|_| Value::Null),
        },
        "DELETE" => rest.delete::<Value>(&path).await,
        other => return Err(format!("Неизвестный метод: {other}")),
    };
    result.map_err(|e| humanize_api_error(&e.to_string()))
}

/// Человекочастные сообщения для типовых ошибок управления сервером.
fn humanize_api_error(raw: &str) -> String {
    if raw.contains("403") {
        return format!("{raw} — не хватает прав (проверь свою роль и иерархию)");
    }
    if raw.contains("401") {
        return format!("{raw} — сессия сброшена, войди заново");
    }
    raw.to_string()
}

/* ========================== PLUGINS =========================== */

fn plugins_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(es)?.join("plugins");
    std::fs::create_dir_all(&dir).map_err(es)?;
    Ok(dir)
}

/// plugins.json: { "file.js": { "enabled": bool, "meta": {...} } }
fn plugins_index_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(plugins_dir(app)?.join("plugins.json"))
}

fn read_plugins_index(app: &tauri::AppHandle) -> Value {
    plugins_index_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_plugins_index(app: &tauri::AppHandle, idx: &Value) -> Result<(), String> {
    let path = plugins_index_path(app)?;
    std::fs::write(&path, serde_json::to_string_pretty(idx).map_err(es)?).map_err(es)
}

/// Валидное имя файла плагина: [a-zA-Z0-9_-]+.js
fn safe_plugin_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let base = name.strip_suffix(".js").unwrap_or(name);
    if base.is_empty()
        || !base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Имя плагина: только латиница, цифры, - и _".into());
    }
    Ok(format!("{base}.js"))
}

#[tauri::command]
pub async fn plugin_list(app: tauri::AppHandle) -> Result<Value, String> {
    let dir = plugins_dir(&app)?;
    let idx = read_plugins_index(&app);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(es)? {
        let Ok(entry) = entry else { continue };
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.ends_with(".js") || file_name == "plugins.json" {
            continue;
        }
        let entry_idx = idx.get(&file_name).cloned().unwrap_or(Value::Null);
        out.push(json!({
            "file": file_name,
            "enabled": entry_idx.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
            "meta": entry_idx.get("meta").cloned().unwrap_or(Value::Null),
            "size": entry.metadata().map(|m| m.len()).unwrap_or(0),
        }));
    }
    out.sort_by(|a, b| {
        a["file"]
            .as_str()
            .unwrap_or("")
            .cmp(b["file"].as_str().unwrap_or(""))
    });
    Ok(Value::Array(out))
}

#[tauri::command]
pub async fn plugin_save(
    app: tauri::AppHandle,
    name: String,
    code: String,
) -> Result<Value, String> {
    let file = safe_plugin_name(&name)?;
    let dir = plugins_dir(&app)?;
    std::fs::write(dir.join(&file), code).map_err(es)?;
    let mut idx = read_plugins_index(&app);
    // Новый/обновлённый плагин по умолчанию выключен — включит пользователь.
    if idx.get(&file).is_none() {
        idx[file.as_str()] = json!({ "enabled": false, "meta": null });
    }
    write_plugins_index(&app, &idx)?;
    Ok(json!({ "file": file, "saved": true }))
}

#[tauri::command]
pub async fn plugin_delete(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let file = safe_plugin_name(&name)?;
    let dir = plugins_dir(&app)?;
    let target = dir.join(&file);
    if target.exists() {
        std::fs::remove_file(&target).map_err(es)?;
    }
    let mut idx = read_plugins_index(&app);
    if let Some(obj) = idx.as_object_mut() {
        obj.remove(&file);
    }
    write_plugins_index(&app, &idx)?;
    Ok(())
}

#[tauri::command]
pub async fn plugin_set_enabled(
    app: tauri::AppHandle,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let file = safe_plugin_name(&name)?;
    let dir = plugins_dir(&app)?;
    if !dir.join(&file).exists() {
        return Err(format!("Плагин {file} не найден"));
    }
    let mut idx = read_plugins_index(&app);
    let entry = idx
        .as_object_mut()
        .ok_or_else(|| "битый plugins.json".to_string())?
        .entry(file.clone())
        .or_insert(json!({ "enabled": false, "meta": null }));
    entry["enabled"] = json!(enabled);
    write_plugins_index(&app, &idx)?;
    Ok(())
}

/// Фронт после исполнения кода плагина присылает его манифест (name,
/// version, author, description) — сохраняем, чтобы список показывал
/// человекочитаемые карточки без исполнения JS.
#[tauri::command]
pub async fn plugin_set_meta(
    app: tauri::AppHandle,
    name: String,
    meta: Value,
) -> Result<(), String> {
    let file = safe_plugin_name(&name)?;
    let mut idx = read_plugins_index(&app);
    let entry = idx
        .as_object_mut()
        .ok_or_else(|| "битый plugins.json".to_string())?
        .entry(file.clone())
        .or_insert(json!({ "enabled": false, "meta": null }));
    entry["meta"] = meta;
    write_plugins_index(&app, &idx)?;
    Ok(())
}

/// Прочитать код плагина (например, для редактирования в UI).
#[tauri::command]
pub async fn plugin_read(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let file = safe_plugin_name(&name)?;
    let dir = plugins_dir(&app)?;
    std::fs::read_to_string(dir.join(&file)).map_err(|e| format!("Плагин {file}: {e}"))
}

/* ====================== MESSENGER / CORE ====================== */

/// Полный кэш-снапшот (используется для восстановления после hot-reload).
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
        "relationships": cache.relationships.values().collect::<Vec<_>>(),
        "friend_presences": cache.friend_presences.values().collect::<Vec<_>>(),
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

/// Начать личную беседу с пользователем (юзер-аккаунт): POST
/// /users/@me/channels {recipient_id}. Возвращает существующий DM-канал,
/// если переписка уже была — Discord идемпотентен к получателю.
#[tauri::command]
pub async fn create_dm(
    state: State<'_, Arc<AppState>>,
    recipient_id: String,
) -> Result<Channel, String> {
    let rest = rest_of(&state)?;
    let ch: Channel = rest
        .post_json(
            "/users/@me/channels",
            json!({ "recipient_id": recipient_id }),
        )
        .await
        .map_err(|e| {
            let m = e.to_string();
            if m.contains("403") {
                format!(
                    "{m} — нельзя писать этому пользователю (нет общих серверов и он не в друзьях)"
                )
            } else {
                m
            }
        })?;
    let mut cache = state.cache.write().unwrap();
    if let Some(slot) = cache.dms.iter_mut().find(|x| x.id == ch.id) {
        // освежим получателей (актуальные ники/аватары)
        *slot = ch.clone();
    } else {
        cache.dms.push(ch.clone());
    }
    Ok(ch)
}

/// Синхронизация друзей/заявок/блок-листа через REST — надёжный
/// fallback, если READY-массив отношений где-то потерялся (resume и т.п.).
#[tauri::command]
pub async fn list_relationships(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<Relationship>, String> {
    let rest = rest_of(&state)?;
    let rels: Vec<Relationship> = rest
        .get_json("/users/@me/relationships")
        .await
        .map_err(es)?;
    {
        let mut cache = state.cache.write().unwrap();
        for r in &rels {
            cache.relationships.insert(r.user.id.clone(), r.clone());
        }
    }
    Ok(rels)
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
    // Желаемое состояние мьюта на случай, если сессия стартует раньше,
    // чем придёт первое эхо VOICE_STATE_UPDATE (events.rs обновляет её дальше).
    {
        let desired = if channel_id.is_some() {
            (self_mute, self_deaf)
        } else {
            (false, false)
        };
        state
            .voice_desired
            .lock()
            .unwrap()
            .insert(guild_id.clone(), desired);
    }
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
