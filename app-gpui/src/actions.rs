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
pub fn trigger_typing(model: &AppModel) {
    let Screen::Main(m) = &model.screen else {
        return;
    };
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

async fn send_gateway(core: &Arc<AppState>, cmd: GatewayCommand) -> Result<(), String> {
    let tx = core
        .gateway_tx
        .read()
        .unwrap()
        .clone()
        .ok_or("Gateway не подключён")?;
    tx.send(cmd).await.map_err(|e| e.to_string())
}
