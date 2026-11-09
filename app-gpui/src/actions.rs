//! Асинхронные действия UI: спавнят задачи на tokio-рантайме, результат
//! возвращают в UI через канал (ui://…), где pump раздаёт их в AppModel.

use gpui::AppContext as _;
use std::sync::Arc;

use rustycord_core::rest::models::Message;
use rustycord_core::state::{AppState, GatewayCommand};
use serde_json::{json, Value};

use crate::bridge::ui_emit;
use crate::state::{AppModel, LoginState, LoginTab, MainState, Screen};

/// Подключиться по токену (кнопка «Войти» / QR done).
pub fn connect_token(model: &mut AppModel, token: String, cx: &mut gpui::Context<AppModel>) {
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    if let Screen::Login(login) = &mut model.screen {
        login.busy = true;
        login.error = None;
    }
    cx.notify();
    crate::bridge::spawn_tokio(async move {
        match rustycord_core::session::connect(&core, &token).await {
            Ok(me) => ui_emit(
                &tx,
                "ui://login-token",
                serde_json::to_value(&me).unwrap_or(Value::Null),
            ),
            Err(e) => ui_emit(&tx, "ui://login-token", json!({ "error": e })),
        }
    });
}

/// Свежий экран логина (после logout / первого запуска).
pub fn new_login_state(cx: &mut gpui::Context<AppModel>) -> Box<LoginState> {
    Box::new(LoginState {
        tab: LoginTab::Token,
        token_input: cx
            .new(|cx| crate::text_input::TextInput::new(cx, "Токен пользователя или бота")),
        email_input: cx.new(|cx| crate::text_input::TextInput::new(cx, "email@example.com")),
        pass_input: cx.new(|cx| {
            let mut f = crate::text_input::TextInput::new(cx, "Пароль");
            f.masked = true;
            f
        }),
        mfa_input: cx.new(|cx| crate::text_input::TextInput::new(cx, "Код из приложения (2FA)")),
        mfa_ticket: None,
        busy: false,
        error: None,
        qr: crate::state::QrStage::Idle,
    })
}

/// Успешное подключение — переключить экран на главный.
pub fn enter_main(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    model.screen = Screen::Main(Box::new(MainState::new(cx)));
    cx.notify();
}

/// Выйти из аккаунта → экран логина.
pub fn logout(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let core = model.core.clone();
    crate::bridge::spawn_tokio(async move {
        rustycord_core::session::disconnect(&core).await;
    });
    model.screen = Screen::Login(new_login_state(cx));
    cx.notify();
}

/// Вход по email+паролю (шаг 1).
pub fn login_email(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Login(login) = &mut model.screen else {
        return;
    };
    if login.busy {
        return;
    }
    let email = login.email_input.read(cx).text();
    let pass = login.pass_input.read(cx).text();
    if email.trim().is_empty() || pass.is_empty() {
        login.error = Some("Заполни email и пароль".into());
        cx.notify();
        return;
    }
    let solver = model.core.captcha.clone();
    let tx = model.ui_tx.clone();
    login.busy = true;
    login.error = None;
    cx.notify();
    crate::bridge::spawn_tokio(async move {
        match rustycord_core::auth::login::login(solver.as_ref(), &email, &pass).await {
            Ok(rustycord_core::auth::login::LoginOutcome::Token(token)) => ui_emit(
                &tx,
                "ui://login-email",
                json!({ "status": "ok", "token": token }),
            ),
            Ok(rustycord_core::auth::login::LoginOutcome::Mfa { ticket, sms }) => ui_emit(
                &tx,
                "ui://login-email",
                json!({ "status": "mfa", "ticket": ticket, "sms": sms }),
            ),
            Err(e) => ui_emit(
                &tx,
                "ui://login-email",
                json!({ "status": "error", "error": format!("{e:#}") }),
            ),
        }
    });
}

/// Второй шаг 2FA.
pub fn login_mfa(model: &mut AppModel, code: String, cx: &mut gpui::Context<AppModel>) {
    let Screen::Login(login) = &mut model.screen else {
        return;
    };
    let Some((ticket, _sms)) = login.mfa_ticket.clone() else {
        return;
    };
    if login.busy {
        return;
    }
    let solver = model.core.captcha.clone();
    let tx = model.ui_tx.clone();
    login.busy = true;
    login.error = None;
    cx.notify();
    crate::bridge::spawn_tokio(async move {
        match rustycord_core::auth::login::submit_mfa(solver.as_ref(), &code, &ticket).await {
            Ok(rustycord_core::auth::login::LoginOutcome::Token(token)) => ui_emit(
                &tx,
                "ui://login-mfa",
                json!({ "status": "ok", "token": token }),
            ),
            Ok(_) => ui_emit(
                &tx,
                "ui://login-mfa",
                json!({ "status": "error", "error": "Неожиданный ответ 2FA" }),
            ),
            Err(e) => ui_emit(
                &tx,
                "ui://login-mfa",
                json!({ "status": "error", "error": format!("{e:#}") }),
            ),
        }
    });
}

/// Старт/сброс QR-сессии.
pub fn qr_toggle(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let core = model.core.clone();
    let Screen::Login(login) = &mut model.screen else {
        return;
    };
    match login.qr {
        crate::state::QrStage::Idle
        | crate::state::QrStage::Failed(_)
        | crate::state::QrStage::Scanned { .. } => {
            login.qr = crate::state::QrStage::Connecting;
            login.error = None;
            crate::bridge::spawn_tokio(async move {
                if let Err(e) = rustycord_core::session::start_remote_auth(&core).await {
                    tracing::warn!("remote-auth: {e:#}");
                }
            });
        }
        _ => {
            login.qr = crate::state::QrStage::Idle;
            crate::bridge::spawn_tokio(async move {
                let _ = rustycord_core::session::cancel_remote_auth(&core).await;
            });
        }
    }
    cx.notify();
}

/* --------------------------- Главный экран --------------------------- */

async fn load_history(
    core: &Arc<AppState>,
    channel_id: &str,
    before: Option<String>,
) -> Result<Vec<Message>, String> {
    let rest = core.rest()?;
    let mut query: Vec<(String, String)> = vec![("limit".into(), "50".into())];
    if let Some(b) = before {
        query.push(("before".into(), b));
    }
    let path = format!("/channels/{channel_id}/messages");
    rest.get_json_q(&path, &query)
        .await
        .map_err(|e| e.to_string())
}

/// Применить результат загрузки истории (ui://history / ui://history-error).
pub fn apply_history(
    model: &mut AppModel,
    channel_id: &str,
    messages: Option<Vec<Message>>,
    error: Option<String>,
    cx: &mut gpui::Context<AppModel>,
) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    m.loading_history = false;
    let history_empty = m
        .messages
        .get(channel_id)
        .map(Vec::is_empty)
        .unwrap_or(true);
    match (messages, error) {
        (Some(list), _) => {
            let entry = m.messages.entry(channel_id.to_string()).or_default();
            let mut merged = list;
            merged.append(entry);
            if merged.len() > 500 {
                merged.drain(..merged.len() - 500);
            }
            *entry = merged;
        }
        // Первая загрузка не удалась — покажем в баннере, история пустая.
        (None, Some(e)) if history_empty => {
            m.banner = Some(format!("История не загрузилась: {e}"));
        }
        _ => {}
    }
    cx.notify();
}

/// Выбор канала: подписки op 37 + история + сброс unreads.
pub fn select_channel(model: &mut AppModel, channel_id: &str, cx: &mut gpui::Context<AppModel>) {
    {
        let Screen::Main(m) = &mut model.screen else {
            return;
        };
        let _guild_id = m.selected_guild.clone();
        m.selected_channel = Some(channel_id.to_string());
        m.unread.remove(channel_id);
        m.reply_to = None;
        m.send_error = None;
    }
    let is_bot = model.core.is_bot.load(std::sync::atomic::Ordering::SeqCst);

    // Подписки (как в Tauri-фронте): гильдия op 37 (пресенсы/typing),
    // затем ленивый список участников канала [[0,99]].
    let core = model.core.clone();
    let (gid, cid) = {
        let Screen::Main(m) = &model.screen else {
            return;
        };
        (m.selected_guild.clone(), channel_id.to_string())
    };
    crate::bridge::spawn_tokio(async move {
        if !is_bot {
            if let Some(g) = gid.clone() {
                let _ = send_gateway(
                    &core,
                    GatewayCommand::BulkGuildSubscribe { guild_ids: vec![g] },
                )
                .await;
            }
            let _ = send_gateway(
                &core,
                GatewayCommand::SubscribeMemberList {
                    guild_id: gid.unwrap_or_else(|| cid.clone()),
                    channel_id: Some(cid.clone()),
                    ranges: vec![(0, 99)],
                },
            )
            .await;
        } else if let Some(g) = gid {
            let _ = send_gateway(&core, GatewayCommand::RequestMembers { guild_id: g }).await;
        }
    });

    // История сообщений (первая загрузка).
    let need_load = {
        let Screen::Main(m) = &mut model.screen else {
            return;
        };
        let need = !m.messages.contains_key(channel_id);
        if need {
            m.loading_history = true;
        }
        need
    };
    if need_load {
        let core = model.core.clone();
        let tx = model.ui_tx.clone();
        let cid = channel_id.to_string();
        crate::bridge::spawn_tokio(async move {
            match load_history(&core, &cid, None).await {
                Ok(list) => ui_emit(
                    &tx,
                    "ui://history",
                    json!({ "channel_id": cid, "messages": list }),
                ),
                Err(e) => ui_emit(
                    &tx,
                    "ui://history-error",
                    json!({ "channel_id": cid, "error": e }),
                ),
            }
        });
    }
    cx.notify();
}

/// Выбор гильдии (или «Домой»): подписка гильдии, сброс канала.
pub fn select_guild(
    model: &mut AppModel,
    guild_id: Option<String>,
    cx: &mut gpui::Context<AppModel>,
) {
    {
        let Screen::Main(m) = &mut model.screen else {
            return;
        };
        m.selected_guild = guild_id.clone();
        m.selected_channel = None;
        m.reply_to = None;
    }
    // op 37 для пресенсов гильдии — сразу (список каналов приедет в
    // GUILD_CREATE, который это подписка и провоцирует).
    if let Some(gid) = guild_id {
        let core = model.core.clone();
        let already = model.core.subscribed_guilds.lock().unwrap().contains(&gid);
        if !already {
            crate::bridge::spawn_tokio(async move {
                let _ = send_gateway(
                    &core,
                    GatewayCommand::BulkGuildSubscribe {
                        guild_ids: vec![gid],
                    },
                )
                .await;
            });
        }
    }
    cx.notify();
}

/// Подгрузить более старые сообщения текущего канала.
pub fn load_older(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    let Some(list) = m.messages.get(&cid) else {
        return;
    };
    if list.is_empty() || m.loading_history {
        return;
    }
    let before = list.first().unwrap().id.clone();
    m.loading_history = true;
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    crate::bridge::spawn_tokio(async move {
        match load_history(&core, &cid, Some(before)).await {
            Ok(list) => ui_emit(
                &tx,
                "ui://history",
                json!({ "channel_id": cid, "messages": list }),
            ),
            Err(e) => ui_emit(
                &tx,
                "ui://history-error",
                json!({ "channel_id": cid, "error": e }),
            ),
        }
    });
    cx.notify();
}

/// Отправка сообщения (Enter в композере или кнопка).
pub fn send_message(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    let composer = m.composer.clone();
    let text = composer.read(cx).text();
    if text.trim().is_empty() {
        return;
    }
    let reply = m.reply_to.as_ref().map(|r| r.id.clone());
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    composer.update(cx, |f, cx| f.clear(cx));
    m.reply_to = None;
    crate::bridge::spawn_tokio(async move {
        let rest = match core.rest() {
            Ok(r) => r,
            Err(e) => {
                ui_emit(&tx, "ui://send-error", json!({ "error": e }));
                return;
            }
        };
        let path = format!("/channels/{cid}/messages");
        let mut body = json!({ "content": text, "tts": false });
        if let Some(reply_id) = reply {
            body["message_reference"] = json!({ "message_id": reply_id });
            body["allowed_mentions"] = json!({ "parse": ["users", "roles", "everyone"] });
        }
        if let Err(e) = rest.post_json::<Value>(&path, body).await {
            ui_emit(&tx, "ui://send-error", json!({ "error": e.to_string() }));
        }
    });
    cx.notify();
}

/// Реакция на сообщение (клик по чипу — toggle своей реакции).
pub fn toggle_reaction(
    model: &mut AppModel,
    message_id: &str,
    emoji: &str,
    cx: &mut gpui::Context<AppModel>,
) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    // Есть ли уже наша реакция на этом сообщении?
    let mine = m
        .messages
        .get(&cid)
        .and_then(|list| list.iter().find(|x| x.id == message_id))
        .and_then(|msg| msg.reactions.as_ref())
        .map(|rs| {
            rs.iter()
                .any(|r| r.me && r.emoji.name.as_deref() == Some(emoji))
        })
        .unwrap_or(false);
    let core = model.core.clone();
    let emoji = emoji.to_string();
    let mid = message_id.to_string();
    crate::bridge::spawn_tokio(async move {
        let Ok(rest) = core.rest() else { return };
        let path = format!(
            "/channels/{cid}/messages/{mid}/reactions/{}/@me",
            crate::urlencode(&emoji)
        );
        if mine {
            let _ = rest.delete::<Value>(&path).await;
        } else {
            let _ = rest.put_empty(&path).await;
        }
    });
    cx.notify();
}

/// Голос: клик по голосовому каналу (вход/выход).
pub fn voice_join_leave(
    model: &mut AppModel,
    guild_id: Option<String>,
    channel_id: Option<String>,
    cx: &mut gpui::Context<AppModel>,
) {
    let core = model.core.clone();
    crate::bridge::spawn_tokio(async move {
        let _ =
            rustycord_core::session::join_voice(&core, guild_id, channel_id, false, false, false);
    });
    cx.notify();
}

/// Переключить мьют/деафен (op4 с текущим каналом).
pub fn voice_toggle(model: &mut AppModel, mute: bool, cx: &mut gpui::Context<AppModel>) {
    let core = model.core.clone();
    let target = core.voice_target.lock().unwrap().clone();
    let Some(t) = target else { return };
    let (cur_mute, cur_deaf) = {
        let key = t.guild_id.clone().unwrap_or_else(|| t.channel_id.clone());
        core.voice_desired
            .lock()
            .unwrap()
            .get(&key)
            .copied()
            .unwrap_or((t.self_mute, t.self_deaf))
    };
    let (nm, nd) = if mute {
        (!cur_mute, cur_deaf)
    } else {
        (cur_mute, !cur_deaf)
    };
    crate::bridge::spawn_tokio(async move {
        let _ = rustycord_core::session::join_voice(
            &core,
            t.guild_id.clone(),
            Some(t.channel_id.clone()),
            nm,
            nd,
            t.self_video,
        );
    });
    cx.notify();
}

/// Триггер typing (перед отправкой, как официальный клиент).
/// Silent Typing (нативный порт плагина): флаг глушит отправку.
pub fn trigger_typing(model: &AppModel) {
    let Screen::Main(m) = &model.screen else {
        return;
    };
    if m.suppress_typing {
        return;
    }
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    let core = model.core.clone();
    crate::bridge::spawn_tokio(async move {
        if let Ok(rest) = core.rest() {
            let _ = rest.post_empty(&format!("/channels/{cid}/typing")).await;
        }
    });
}

/* ------------- Редактирование/удаление/вложения (паритет с Tauri) ------------- */

/// Начать редактирование своего сообщения: текст — в композер,
/// поверх — баннер с кнопкой отмены; Enter = сохранить.
pub fn edit_start(model: &mut AppModel, message_id: &str, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    let Some(msg) = m
        .messages
        .get(&cid)
        .and_then(|list| list.iter().find(|x| x.id == message_id))
        .cloned()
    else {
        return;
    };
    m.edit_target = Some((msg.id.clone(), msg.content.clone()));
    m.reply_to = None;
    m.composer.update(cx, |f, cx| f.set_text(&msg.content, cx));
    cx.notify();
}

/// Отменить редактирование (кнопка «Отмена» или смена канала).
pub fn edit_cancel(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    if let Screen::Main(m) = &mut model.screen {
        m.edit_target = None;
    }
    if let Screen::Main(m) = &model.screen {
        m.composer.update(cx, |f, cx| f.clear(cx));
    }
    cx.notify();
}

/// Сохранить правку (PATCH): обновление приедет MESSAGE_UPDATE dispatch'ем.
pub fn edit_save(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    let Some((mid, orig)) = m.edit_target.clone() else {
        return;
    };
    let composer = m.composer.clone();
    let text = composer.read(cx).text();
    if text == orig {
        // Ничего не изменилось — просто выходим из режима.
        edit_cancel(model, cx);
        return;
    }
    if text.trim().is_empty() {
        if let Screen::Main(m) = &mut model.screen {
            m.send_error = Some("Пустое сообщение нельзя сохранить — удали его 🗑".into());
        }
        cx.notify();
        return;
    }
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    if let Screen::Main(m) = &mut model.screen {
        m.edit_target = None;
    }
    composer.update(cx, |f, cx| f.clear(cx));
    crate::bridge::spawn_tokio(async move {
        let Ok(rest) = core.rest() else { return };
        let path = format!("/channels/{cid}/messages/{mid}");
        if let Err(e) = rest
            .patch_json::<Value>(&path, json!({ "content": text }))
            .await
        {
            ui_emit(&tx, "ui://send-error", json!({ "error": e.to_string() }));
        }
    });
    cx.notify();
}

/// Удалить сообщение (права проверит сервер): локально уберёт MESSAGE_DELETE.
pub fn delete_message(model: &mut AppModel, message_id: &str, _cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    let mid = message_id.to_string();
    crate::bridge::spawn_tokio(async move {
        let Ok(rest) = core.rest() else { return };
        let path = format!("/channels/{cid}/messages/{mid}");
        if let Err(e) = rest.delete::<Value>(&path).await {
            ui_emit(
                &tx,
                "ui://banner",
                json!({ "text": format!("Удаление: {e}") }),
            );
        }
    });
}

/// Прикрепить файлы (📎): нативный диалог → multipart POST.
/// Текст композера едет тем же запросом (как в официальном клиенте).
pub fn upload_pick(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(cid) = m.selected_channel.clone() else {
        return;
    };
    if m.uploading {
        return;
    }
    let composer = m.composer.clone();
    let content = composer.read(cx).text();
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    if let Screen::Main(m) = &mut model.screen {
        m.uploading = true;
    }
    cx.notify();
    crate::bridge::spawn_tokio(async move {
        #[cfg(feature = "file-dialogs")]
        {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Прикрепить файлы (до 10, максимум 25 МБ каждая)")
                .pick_files()
                .await;
            upload_flow(core, tx, cid, content, picked).await;
        }
        #[cfg(not(feature = "file-dialogs"))]
        {
            let _ = (core, cid, content);
            ui_emit(
                &tx,
                "ui://send-error",
                json!({ "error": "Файловый диалог недоступен в этой сборке (соберите с file-dialogs)" }),
            );
            ui_emit(&tx, "ui://upload-done", json!({}));
        }
    });
}

/// Отмена диалога → тихо вернуть композер в строй; файлы → multipart.
#[cfg(feature = "file-dialogs")]
async fn upload_flow(
    core: Arc<AppState>,
    tx: crate::bridge::EventTx,
    cid: String,
    content: String,
    picked: Option<Vec<rfd::FileHandle>>,
) {
    let Some(handles) = picked else {
        ui_emit(&tx, "ui://upload-done", json!({}));
        return;
    };
    if handles.is_empty() {
        ui_emit(&tx, "ui://upload-done", json!({}));
        return;
    }
    if handles.len() > 10 {
        ui_emit(
            &tx,
            "ui://send-error",
            json!({ "error": "Максимум 10 файлов за раз (как в оригинале)" }),
        );
        ui_emit(&tx, "ui://upload-done", json!({}));
        return;
    }
    let mut files: Vec<(String, String, Vec<u8>)> = Vec::with_capacity(handles.len());
    for h in handles {
        let name = h.file_name();
        let bytes = match tokio::fs::read(h.path()).await {
            Ok(b) => b,
            Err(e) => {
                ui_emit(
                    &tx,
                    "ui://send-error",
                    json!({ "error": format!("Не прочитать {name}: {e}") }),
                );
                ui_emit(&tx, "ui://upload-done", json!({}));
                return;
            }
        };
        if bytes.len() > 25 * 1024 * 1024 {
            ui_emit(
                &tx,
                "ui://send-error",
                json!({ "error": format!(
                    "{name}: больше 25 МБ (лимит загрузки Discord — 25 МБ без Nitro)"
                ) }),
            );
            ui_emit(&tx, "ui://upload-done", json!({}));
            return;
        }
        let mime = rustycord_core::rest::client::mime_for(&name);
        files.push((name, mime, bytes));
    }
    // Файлы на месте — текст композера уезжает с запросом: чистим поле.
    ui_emit(&tx, "ui://upload-picked", json!({}));
    let Ok(rest) = core.rest() else {
        ui_emit(&tx, "ui://upload-done", json!({}));
        return;
    };
    let path = format!("/channels/{cid}/messages");
    let content_opt = if content.trim().is_empty() {
        None
    } else {
        Some(content.as_str())
    };
    // Сообщение приедет обычным MESSAGE_CREATE dispatch'ем.
    if let Err(e) = rest
        .post_multipart::<Value>(&path, content_opt, &files)
        .await
    {
        ui_emit(&tx, "ui://send-error", json!({ "error": e.to_string() }));
    }
    ui_emit(&tx, "ui://upload-done", json!({}));
}

/* --------------------------- ЛС и модерация --------------------------- */

/// Открыть (или создать) ЛС с пользователем: POST /users/@me/channels.
pub fn create_dm(model: &mut AppModel, recipient_id: &str, cx: &mut gpui::Context<AppModel>) {
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    let uid = recipient_id.to_string();
    crate::bridge::spawn_tokio(async move {
        let Ok(rest) = core.rest() else { return };
        match rest
            .post_json::<rustycord_core::rest::models::Channel>(
                "/users/@me/channels",
                json!({ "recipient_id": uid }),
            )
            .await
        {
            Ok(ch) => {
                // Кэш ядра — как в Tauri-команде: освежить или добавить.
                {
                    let mut cache = core.cache.write().unwrap();
                    match cache.dms.iter_mut().find(|x| x.id == ch.id) {
                        Some(slot) => *slot = ch.clone(),
                        None => cache.dms.push(ch.clone()),
                    }
                }
                ui_emit(&tx, "ui://dm-created", json!({ "channel_id": ch.id }));
            }
            Err(e) => {
                let m = e.to_string();
                let hint = if m.contains("403") {
                    format!("{m} — нельзя писать этому пользователю (нет общих серверов и он не в друзьях)")
                } else {
                    m
                };
                ui_emit(&tx, "ui://banner", json!({ "text": hint }));
            }
        }
    });
    cx.notify();
}

/// Кик/бан участника (кнопки в списке участников; первый клик — подтвердить,
/// второй — исполнить). Права проверяет сервер.
pub fn member_action(
    model: &mut AppModel,
    user_id: &str,
    action: crate::state::ModAction,
    cx: &mut gpui::Context<AppModel>,
) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(gid) = m.selected_guild.clone() else {
        return;
    };
    let uid = user_id.to_string();
    if m.confirm_mod.as_ref() != Some(&(uid.clone(), action)) {
        m.confirm_mod = Some((uid, action));
        cx.notify();
        return;
    }
    m.confirm_mod = None;
    let core = model.core.clone();
    let tx = model.ui_tx.clone();
    crate::bridge::spawn_tokio(async move {
        let Ok(rest) = core.rest() else { return };
        let res: Result<(), anyhow::Error> = match action {
            crate::state::ModAction::Kick => rest
                .delete::<Value>(&format!("/guilds/{gid}/members/{uid}"))
                .await
                .map(|_| ()),
            crate::state::ModAction::Ban => {
                rest.put_empty(&format!("/guilds/{gid}/bans/{uid}")).await
            }
        };
        if let Err(e) = res {
            let m = e.to_string();
            let hint = if m.contains("403") {
                format!("{m} — не хватает прав (и роль должна быть выше цели)")
            } else {
                m
            };
            ui_emit(&tx, "ui://banner", json!({ "text": hint }));
        }
    });
    cx.notify();
}

/* ------------------------ Стримы (Go Live) и камера ------------------------ */

/// Текущие флаги мьюта/деафена для op 4 (как voice_toggle).
fn current_voice_flags(core: &Arc<AppState>) -> (bool, bool) {
    let target = core.voice_target.lock().unwrap().clone();
    let Some(t) = target else {
        return (false, false);
    };
    let key = t.guild_id.clone().unwrap_or_else(|| t.channel_id.clone());
    core.voice_desired
        .lock()
        .unwrap()
        .get(&key)
        .copied()
        .unwrap_or((t.self_mute, t.self_deaf))
}

/// «Показать экран» (Go Live): op 18 + окно превью (wry). Повторный клик
/// или закрытие превью — op 19 по stream_key из STREAM_CREATE.
pub fn stream_toggle(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    if let Some(key) = m.my_stream_key.clone() {
        m.my_stream_key = None;
        let core = model.core.clone();
        crate::bridge::spawn_tokio(async move {
            let _ = send_gateway(&core, GatewayCommand::StreamDelete { stream_key: key }).await;
        });
        cx.notify();
        return;
    }
    // Голос должен быть активен — трансляция живёт в голосовом канале.
    let target = model.core.voice_target.lock().unwrap().clone();
    let Some(t) = target else {
        m.banner = Some("Сначала войди в голосовой канал".into());
        cx.notify();
        return;
    };
    let core = model.core.clone();
    let (gid, cid) = (t.guild_id.clone(), t.channel_id.clone());
    crate::bridge::spawn_tokio(async move {
        let _ = send_gateway(
            &core,
            GatewayCommand::StreamCreate {
                guild_id: gid,
                channel_id: cid,
            },
        )
        .await;
    });
    // Локальное превью — тот же UX, что в Tauri (getDisplayMedia в webview).
    #[cfg(feature = "captcha-webview")]
    {
        let tx = model.ui_tx.clone();
        crate::preview::open(
            crate::preview::PreviewKind::Screen,
            Box::new(move || {
                let _ = tx.send(("ui://preview-stopped".into(), json!({ "kind": "screen" })));
            }),
        );
    }
    cx.notify();
}

/// Остановить свою трансляцию (из превью-окна или по кнопке).
pub fn stop_my_stream(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    if let Some(key) = m.my_stream_key.take() {
        let core = model.core.clone();
        crate::bridge::spawn_tokio(async move {
            let _ = send_gateway(&core, GatewayCommand::StreamDelete { stream_key: key }).await;
        });
    }
    cx.notify();
}

/// Пауза трансляции (op 22): картинка замирает у зрителей, звук живёт.
pub fn stream_pause_toggle(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let Some(key) = m.my_stream_key.clone() else {
        return;
    };
    let paused = m.streams.get(&key).map(|s| !s.paused).unwrap_or(false); // нет инфо — ставим на паузу
    if let Some(s) = m.streams.get_mut(&key) {
        s.paused = paused;
    }
    let core = model.core.clone();
    crate::bridge::spawn_tokio(async move {
        let _ = send_gateway(
            &core,
            GatewayCommand::StreamSetPaused {
                stream_key: key,
                paused,
            },
        )
        .await;
    });
    cx.notify();
}

/// Камера: op 4 с self_video (камера НЕ через op 18 — урок v0.10.3) +
/// окно превью на getUserMedia.
pub fn camera_toggle(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    let Screen::Main(m) = &mut model.screen else {
        return;
    };
    let on = !m.camera_on;
    camera_set(model, on, cx);
}

/// Установить состояние камеры (op 4 self_video=on).
pub fn camera_set(model: &mut AppModel, on: bool, cx: &mut gpui::Context<AppModel>) {
    {
        let Screen::Main(m) = &mut model.screen else {
            return;
        };
        let target = model.core.voice_target.lock().unwrap().clone();
        let Some(t) = target else {
            if on {
                m.banner = Some("Сначала войди в голосовой канал".into());
            }
            cx.notify();
            return;
        };
        m.camera_on = on;
        let core = model.core.clone();
        let (nm, nd) = current_voice_flags(&core);
        let (gid, cid) = (t.guild_id.clone(), t.channel_id.clone());
        crate::bridge::spawn_tokio(async move {
            let _ = rustycord_core::session::join_voice(&core, gid, Some(cid), nm, nd, on);
        });
        if on {
            #[cfg(feature = "captcha-webview")]
            {
                let tx = model.ui_tx.clone();
                crate::preview::open(
                    crate::preview::PreviewKind::Camera,
                    Box::new(move || {
                        let _ =
                            tx.send(("ui://preview-stopped".into(), json!({ "kind": "camera" })));
                    }),
                );
            }
        }
    }
    cx.notify();
}

/* --------------------------- Настройки/обновления --------------------------- */

/// Проверить обновления (GitHub Releases) — раздел «О программе».
pub fn check_updates(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) {
    {
        let Screen::Main(m) = &mut model.screen else {
            return;
        };
        if m.update_status.checking {
            return;
        }
        m.update_status.checking = true;
    }
    let tx = model.ui_tx.clone();
    let current = env!("CARGO_PKG_VERSION").to_string();
    crate::bridge::spawn_tokio(async move {
        match rustycord_core::updater::check(&current).await {
            Ok(info) => {
                let text = if info.no_releases {
                    "Публичных релизов ещё нет (dev-сборка?)".to_string()
                } else if info.is_newer {
                    format!("Доступна {} (у вас {current})", info.latest)
                } else {
                    format!("У вас актуальная версия ({current})")
                };
                ui_emit(
                    &tx,
                    "ui://update-checked",
                    json!({ "text": text, "url": info.url, "is_newer": info.is_newer }),
                );
            }
            Err(e) => ui_emit(
                &tx,
                "ui://update-checked",
                json!({ "text": format!("Не удалось проверить: {e}"), "url": "", "is_newer": false }),
            ),
        }
    });
    cx.notify();
}

/// Лупбек микрофона (слышим себя, как слышат собеседники) — на все сессии.
pub fn loopback_set(model: &AppModel, on: bool) {
    let sessions = model.core.voice_sessions.lock().unwrap();
    if sessions.is_empty() {
        return;
    }
    for h in sessions.values() {
        h.loopback.store(on, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Настройки DSP (шумодав + AGC) — применяются к следующему кадру
/// живой сессии без перезахода в канал.
pub fn dsp_set(model: &AppModel, ns_enabled: bool, ns_strength: &str, agc_enabled: bool) {
    let Some(strength) = rustycord_core::voice::dsp::DspConfig::strength_from_str(ns_strength)
    else {
        return;
    };
    *model.core.voice_dsp_cfg.lock().unwrap() = rustycord_core::voice::dsp::DspConfig {
        ns_enabled,
        ns_strength: strength,
        agc_enabled,
    };
}

async fn send_gateway(core: &Arc<AppState>, cmd: GatewayCommand) -> Result<(), String> {
    let tx = core
        .gateway_tx
        .read()
        .unwrap()
        .clone()
        .ok_or("Gateway не подключён")?;
    tx.send(cmd).await.map_err(|e| e.to_string())
}
