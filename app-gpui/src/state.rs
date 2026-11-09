//! Корневая модель приложения: экраны (логин/главный), локальное UI-состояние
//! (истории сообщений, typing, unreads, голос) и применение событий ядра.
//! Канонические данные (гильдии/каналы/участники/пресенсы/войс) живут в
//! `rustycord_core::state::AppState` — рендер читает их напрямую, здесь
//! только то, чего в кэше ядра нет.

use gpui::AppContext as _;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use rustycord_core::rest::models::{Message, Reaction};
use rustycord_core::state::AppState;
use serde_json::Value;

use crate::bridge::EventTx;

/* ------------------------------ Экраны ------------------------------ */

pub enum Screen {
    Login(Box<LoginState>),
    Main(Box<MainState>),
}

#[derive(Clone, Copy, PartialEq)]
pub enum LoginTab {
    Token,
    Email,
    Qr,
}

/// Стадии QR-логина (события discord://qr).
#[allow(dead_code)]
pub enum QrStage {
    Idle,
    Connecting,
    /// QR-матрица (row-major, true = тёмный модуль) + fingerprint.
    Matrix {
        modules: Vec<bool>,
        width: usize,
        fingerprint: String,
    },
    Scanned {
        username: String,
    },
    Finishing,
    Failed(String),
}

pub struct LoginState {
    pub tab: LoginTab,
    pub token_input: gpui::Entity<crate::text_input::TextInput>,
    pub email_input: gpui::Entity<crate::text_input::TextInput>,
    pub pass_input: gpui::Entity<crate::text_input::TextInput>,
    pub mfa_input: gpui::Entity<crate::text_input::TextInput>,
    /// Активный тикет 2FA (введён email+пароль, ждём код).
    pub mfa_ticket: Option<(String, bool)>, // (ticket, sms)
    pub busy: bool,
    pub error: Option<String>,
    pub qr: QrStage,
}

pub struct MainState {
    /// None → домашний экран (ЛС/друзья).
    pub selected_guild: Option<String>,
    pub selected_channel: Option<String>,
    /// Поле ввода сообщения.
    pub composer: gpui::Entity<crate::text_input::TextInput>,
    /// Истории каналов: channel_id → сообщения (старые → новые), кап 500.
    pub messages: HashMap<String, Vec<Message>>,
    pub loading_history: bool,
    /// channel_id → (user_id → (имя, дедлайн)) — «печатает…».
    pub typing: HashMap<String, HashMap<String, (String, Instant)>>,
    /// channel_id → непрочитанные.
    pub unread: HashMap<String, u32>,
    /// Порядок гильдий рейла (из guild-layout rail_order + прихода GUILD_CREATE).
    pub guild_order: Vec<String>,
    /// Реплай-цель композера.
    pub reply_to: Option<Message>,
    /// Ошибка последней отправки (баннер над композером).
    pub send_error: Option<String>,
    /// Голосовая панель: сессии по guild_id (ключ как в ядре: гильдия или
    /// канал для ЛС-звонков) + говорящие сейчас.
    pub voice_connected: HashMap<String, ()>, // guild_id → ()
    pub speaking: HashSet<String>, // user_id
    pub mic_level: f32,
    /// Информационный баннер (например, «переподключение…»).
    pub banner: Option<String>,
    /// Прокрутить ленту чата вниз при следующем рендере.
    pub scroll_bottom_pending: bool,
}

impl MainState {
    pub fn new(cx: &mut gpui::Context<crate::AppModel>) -> Self {
        Self {
            selected_guild: None,
            selected_channel: None,
            composer: cx.new(|cx| crate::text_input::TextInput::new(cx, "Сообщение…")),
            messages: HashMap::new(),
            loading_history: false,
            typing: HashMap::new(),
            unread: HashMap::new(),
            guild_order: Vec::new(),
            reply_to: None,
            send_error: None,
            voice_connected: HashMap::new(),
            speaking: HashSet::new(),
            mic_level: 0.0,
            banner: None,
            scroll_bottom_pending: true,
        }
    }

    /// Вставить/заменить сообщение в истории канала.
    pub fn upsert_message(&mut self, msg: &Message) {
        let list = self.messages.entry(msg.channel_id.clone()).or_default();
        match list.iter_mut().find(|m| m.id == msg.id) {
            Some(slot) => *slot = msg.clone(),
            None => {
                list.push(msg.clone());
                if list.len() > 500 {
                    list.drain(..list.len() - 500);
                }
            }
        }
    }

    pub fn remove_message(&mut self, channel_id: &str, message_id: &str) {
        if let Some(list) = self.messages.get_mut(channel_id) {
            list.retain(|m| m.id != message_id);
        }
    }
}

/* ----------------------------- AppModel ----------------------------- */

pub struct AppModel {
    pub core: Arc<AppState>,
    pub screen: Screen,
    /// Канал для инъекции результатов UI-команд (ui://…).
    pub ui_tx: EventTx,
    /// Мигание курсора (тикер подключим в следующей итерации).
    #[allow(dead_code)]
    pub blink_on: bool,
    /// Скролл ленты чата (автоскролл вниз при новых сообщениях).
    pub chat_scroll: gpui::ScrollHandle,
}

impl AppModel {
    pub fn new(core: Arc<AppState>, ui_tx: EventTx, cx: &mut gpui::Context<Self>) -> Self {
        Self {
            core,
            screen: Screen::Login(Box::new(LoginState {
                tab: LoginTab::Token,
                token_input: cx
                    .new(|cx| crate::text_input::TextInput::new(cx, "Токен пользователя или бота")),
                email_input: cx
                    .new(|cx| crate::text_input::TextInput::new(cx, "email@example.com")),
                pass_input: cx.new(|cx| {
                    let mut f = crate::text_input::TextInput::new(cx, "Пароль");
                    f.masked = true;
                    f
                }),
                mfa_input: cx
                    .new(|cx| crate::text_input::TextInput::new(cx, "Код из приложения (2FA)")),
                mfa_ticket: None,
                busy: false,
                error: None,
                qr: QrStage::Idle,
            })),
            ui_tx,
            blink_on: true,
            chat_scroll: gpui::ScrollHandle::new(),
        }
    }

    /// Применить событие (discord://… от ядра или ui://… от команд UI).
    pub fn handle_event(&mut self, event: &str, payload: Value, cx: &mut gpui::Context<Self>) {
        match event {
            "discord://status" => self.apply_status(&payload),
            "discord://warning" => {
                if let Some(msg) = payload.get("message").and_then(|v| v.as_str()) {
                    if let Screen::Main(m) = &mut self.screen {
                        m.banner = Some(msg.to_string());
                    }
                }
            }
            "discord://ready" => cx.notify(),
            "discord://guild-layout" => self.apply_guild_layout(&payload),
            "discord://member-list" => cx.notify(),
            "discord://voice" => self.apply_voice(&payload),
            "discord://qr" => self.apply_qr(&payload, cx),
            "discord://dispatch" => self.apply_dispatch(&payload, cx),
            "ui://login-token" => self.ui_login_token(&payload, cx),
            "ui://login-email" => self.ui_login_email(&payload, cx),
            "ui://login-mfa" => self.ui_login_mfa(&payload, cx),
            "ui://history" => {
                let cid = payload
                    .get("channel_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let messages: Option<Vec<Message>> = payload
                    .get("messages")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok());
                crate::actions::apply_history(self, &cid, messages, None, cx);
                self.mark_scroll_bottom(cx);
            }
            "ui://history-error" => {
                let cid = payload
                    .get("channel_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let error = payload
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                crate::actions::apply_history(self, &cid, None, error, cx);
            }
            "ui://send-error" => {
                if let Some(e) = payload.get("error").and_then(|v| v.as_str()) {
                    if let Screen::Main(m) = &mut self.screen {
                        m.send_error = Some(e.to_string());
                    }
                }
                cx.notify();
            }
            _ => cx.notify(),
        }
    }

    /// Запланировать автоскролл вниз (новые сообщения/история загружена).
    fn mark_scroll_bottom(&mut self, cx: &mut gpui::Context<Self>) {
        if let Screen::Main(m) = &mut self.screen {
            m.scroll_bottom_pending = true;
        }
        cx.notify();
    }

    fn apply_status(&mut self, payload: &Value) {
        let stage = payload.get("stage").and_then(|v| v.as_str()).unwrap_or("");
        let detail = payload
            .get("detail")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if let Screen::Main(m) = &mut self.screen {
            m.banner = match stage {
                "connected" | "shutdown" => None,
                other => Some(match detail {
                    Some(d) => format!("{other}: {d}"),
                    None => other.to_string(),
                }),
            };
        }
    }

    /// rail_order: [{type:"guild", id} | {type:"folder", index}] + folders.
    fn apply_guild_layout(&mut self, payload: &Value) {
        let mut order: Vec<String> = Vec::new();
        if let Some(arr) = payload.get("rail_order").and_then(|v| v.as_array()) {
            for item in arr {
                if item.get("type").and_then(|v| v.as_str()) == Some("guild") {
                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                        order.push(id.to_string());
                    }
                }
            }
        }
        if let Screen::Main(m) = &mut self.screen {
            if !order.is_empty() {
                m.guild_order = order;
            }
        }
    }

    fn apply_voice(&mut self, payload: &Value) {
        let kind = payload.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let data = payload.get("data").cloned().unwrap_or(Value::Null);
        if let Screen::Main(m) = &mut self.screen {
            match kind {
                "connected" => {
                    if let Some(gid) = data.get("guild_id").and_then(|v| v.as_str()) {
                        m.voice_connected.insert(gid.to_string(), ());
                    }
                }
                "disconnected" => {
                    if let Some(gid) = data.get("guild_id").and_then(|v| v.as_str()) {
                        m.voice_connected.remove(gid);
                        m.speaking.clear();
                        m.mic_level = 0.0;
                    }
                }
                "speaking" => {
                    let uid = data
                        .get("user_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let speaking = data
                        .get("speaking")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if speaking {
                        m.speaking.insert(uid);
                    } else {
                        m.speaking.remove(&uid);
                    }
                }
                "mic_level" => {
                    // Телеметрия 20 Гц — рендер не дёргаем (индикатор уровня
                    // появится со спец-элементом).
                    m.mic_level = data
                        .get("level")
                        .and_then(|v| v.as_f64())
                        .map(|v| v as f32)
                        .unwrap_or(0.0);
                }
                _ => {}
            }
        }
    }

    fn apply_qr(&mut self, payload: &Value, cx: &mut gpui::Context<Self>) {
        let stage = payload.get("stage").and_then(|v| v.as_str()).unwrap_or("");
        let Screen::Login(login) = &mut self.screen else {
            return;
        };
        match stage {
            "connecting" => login.qr = QrStage::Connecting,
            "fingerprint" => {
                let fp = payload
                    .get("fingerprint")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                match crate::qr::matrix(&format!("https://discord.com/ra/{fp}")) {
                    Ok((modules, width)) => {
                        login.qr = QrStage::Matrix {
                            modules,
                            width,
                            fingerprint: fp,
                        };
                    }
                    Err(e) => login.qr = QrStage::Failed(format!("QR: {e}")),
                }
            }
            "scanned" => {
                let name = payload
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                login.qr = QrStage::Scanned { username: name };
            }
            "done" => {
                login.qr = QrStage::Finishing;
                // Токен пришёл — подключаемся (как кнопка «Войти»).
                if let Some(token) = payload.get("token").and_then(|v| v.as_str()) {
                    crate::actions::connect_token(self, token.to_string(), cx);
                }
            }
            "timeout" => {
                login.qr = QrStage::Failed(
                    payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("QR истёк")
                        .to_string(),
                );
            }
            "cancelled" => {
                login.qr = QrStage::Failed("Вход отменён на телефоне".into());
            }
            "error" => {
                let msg = payload
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ошибка remote-auth");
                login.qr = QrStage::Failed(msg.to_string());
            }
            _ => {}
        }
    }

    fn apply_dispatch(&mut self, payload: &Value, cx: &mut gpui::Context<Self>) {
        let t = payload.get("t").and_then(|v| v.as_str()).unwrap_or("");
        let d = payload.get("d").cloned().unwrap_or(Value::Null);
        match t {
            "MESSAGE_CREATE" => {
                if let Ok(msg) = serde_json::from_value::<Message>(d.clone()) {
                    let is_selected = matches!(
                        &self.screen,
                        Screen::Main(m) if m.selected_channel.as_deref() == Some(msg.channel_id.as_str())
                    );
                    if let Screen::Main(m) = &mut self.screen {
                        // Печатающий перестал печатать — сообщение пришло.
                        if let Some(user_id) = msg.author.as_ref().map(|a| a.id.clone()) {
                            if let Some(ch) = m.typing.get_mut(&msg.channel_id) {
                                ch.remove(&user_id);
                            }
                        }
                        m.upsert_message(&msg);
                        if !is_selected {
                            *m.unread.entry(msg.channel_id.clone()).or_insert(0) += 1;
                        } else {
                            m.scroll_bottom_pending = true;
                        }
                    }
                }
            }
            "MESSAGE_UPDATE" => {
                if let Ok(msg) = serde_json::from_value::<Message>(d) {
                    if let Screen::Main(m) = &mut self.screen {
                        m.upsert_message(&msg);
                    }
                }
            }
            "MESSAGE_DELETE" => {
                let ch = d.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
                let id = d.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if let Screen::Main(m) = &mut self.screen {
                    m.remove_message(ch, id);
                }
            }
            "MESSAGE_DELETE_BULK" => {
                let ch = d.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(ids) = d.get("ids").and_then(|v| v.as_array()) {
                    if let Screen::Main(m) = &mut self.screen {
                        for id in ids.iter().filter_map(|v| v.as_str()) {
                            m.remove_message(ch, id);
                        }
                    }
                }
            }
            "MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" => {
                self.apply_reaction(&d, t.ends_with("ADD"));
            }
            "TYPING_START" => {
                let ch = d
                    .get("channel_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let user = d
                    .get("user")
                    .and_then(|u| {
                        u.get("global_name")
                            .and_then(|v| v.as_str())
                            .or_else(|| u.get("username").and_then(|v| v.as_str()))
                    })
                    .or_else(|| d.get("user_id").and_then(|v| v.as_str()))
                    .unwrap_or("?")
                    .to_string();
                let uid = d
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if let Screen::Main(m) = &mut self.screen {
                    let slot = m.typing.entry(ch).or_default();
                    let key = uid.unwrap_or_else(|| user.clone());
                    slot.insert(
                        key,
                        (user, Instant::now() + std::time::Duration::from_secs(8)),
                    );
                }
            }
            "GUILD_CREATE" => {
                // Порядок рейла: приход гильдии (если ещё не в порядке).
                if let Some(gid) = d.get("id").and_then(|v| v.as_str()) {
                    if let Screen::Main(m) = &mut self.screen {
                        if !m.guild_order.iter().any(|g| g == gid) {
                            m.guild_order.push(gid.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        // Прочие dispatch'ы меняют кэш ядра (каналы/пресенсы/войс) —
        // перерисовка подхватит новые данные сама.
        cx.notify();
    }

    fn apply_reaction(&mut self, d: &Value, add: bool) {
        let Some(ch) = d.get("channel_id").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(mid) = d.get("message_id").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(emoji) = d.get("emoji") else { return };
        let emoji_name = emoji
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let emoji_id = emoji.get("id").and_then(|v| v.as_str()).map(str::to_string);
        let me = d.get("user_id").and_then(|v| v.as_str()) == self.core.self_id().as_deref();
        let Screen::Main(m) = &mut self.screen else {
            return;
        };
        let Some(list) = m.messages.get_mut(ch) else {
            return;
        };
        let Some(msg) = list.iter_mut().find(|x| x.id == mid) else {
            return;
        };
        let reactions = msg.reactions.get_or_insert_with(Vec::new);
        let existing = reactions.iter().position(|r| {
            r.emoji.name.as_deref() == Some(emoji_name.as_str())
                && r.emoji.id.as_deref() == emoji_id.as_deref()
        });
        match (add, existing) {
            (true, Some(i)) => {
                reactions[i].count += 1;
                if me {
                    reactions[i].me = true;
                }
            }
            (true, None) => reactions.push(Reaction {
                count: 1,
                me,
                emoji: rustycord_core::rest::models::Emoji {
                    id: emoji_id,
                    name: Some(emoji_name),
                    animated: emoji
                        .get("animated")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                },
            }),
            (false, Some(i)) => {
                reactions[i].count -= 1;
                if me {
                    reactions[i].me = false;
                }
                if reactions[i].count <= 0 {
                    reactions.remove(i);
                }
            }
            (false, None) => {}
        }
    }

    /* ------------------------- ui:// результаты ------------------------- */

    fn ui_login_token(&mut self, payload: &Value, cx: &mut gpui::Context<Self>) {
        let Screen::Login(login) = &mut self.screen else {
            return;
        };
        login.busy = false;
        if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
            login.error = Some(err.to_string());
            if matches!(login.qr, QrStage::Finishing) {
                login.qr = QrStage::Failed(err.to_string());
            }
            cx.notify();
            return;
        }
        // Успех: session::connect вернул профиль — переключаемся на главный.
        crate::actions::enter_main(self, cx);
    }

    fn ui_login_email(&mut self, payload: &Value, cx: &mut gpui::Context<Self>) {
        let Screen::Login(login) = &mut self.screen else {
            return;
        };
        login.busy = false;
        let status = payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("error");
        match status {
            "mfa" => {
                let ticket = payload
                    .get("ticket")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let sms = payload
                    .get("sms")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                login.mfa_ticket = Some((ticket, sms));
                login.error = None;
            }
            "ok" => {
                // Токен внутри — подключаемся.
                let token = payload
                    .get("token")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                login.busy = true;
                crate::actions::connect_token(self, token, cx);
            }
            _ => {
                login.error = Some(
                    payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("ошибка входа")
                        .to_string(),
                );
            }
        }
        cx.notify();
    }

    fn ui_login_mfa(&mut self, payload: &Value, cx: &mut gpui::Context<Self>) {
        let Screen::Login(login) = &mut self.screen else {
            return;
        };
        login.busy = false;
        let status = payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("error");
        match status {
            "ok" => {
                let token = payload
                    .get("token")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                login.busy = true;
                crate::actions::connect_token(self, token, cx);
            }
            _ => {
                login.error = Some(
                    payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("неверный код 2FA")
                        .to_string(),
                );
            }
        }
        cx.notify();
    }
}
