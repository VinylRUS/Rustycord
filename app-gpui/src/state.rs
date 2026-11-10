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

/* --------------------- Рейл серверов: папки + порядок --------------------- */

/// Папка серверов из раскладки пользователя (user_settings_proto /
/// guild_folders) — как в официальном клиенте.
#[derive(Debug, Clone, PartialEq)]
pub struct GuildFolder {
    pub name: Option<String>,
    /// Цвет папки (int, младшие 24 бита — RGB).
    pub color: Option<i64>,
    pub guild_ids: Vec<String>,
}

/// Элемент рейла: отдельный сервер или папка (индекс в `folders`).
/// Порядок точный (interleaving из события ядра) — та же схема, что
/// в Tauri-фронтенде (ServerRail.svelte) и в официальном клиенте.
#[derive(Debug, Clone, PartialEq)]
pub enum RailItem {
    Guild(String),
    Folder(usize),
}

/// Секции экрана настроек (оверлей над главным экраном).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Voice,
    Behavior,
    Account,
    Diagnostics,
    About,
}

/// Живая трансляция Go Live (сигналинг op 18/19/22; d обогащён ядром —
/// guild_id/channel_id/user_id распарсены из stream_key).
#[derive(Clone)]
pub struct StreamUi {
    pub stream_key: String,
    pub user_id: String,
    pub guild_id: Option<String>,
    pub channel_id: String,
    pub paused: bool,
    pub viewers: u64,
}

/// Модерация участника (контекстные кнопки в списке участников).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModAction {
    Kick,
    Ban,
}

/// Результат проверки обновлений (для раздела «О программе»).
#[derive(Clone)]
pub struct UpdateStatus {
    pub checking: bool,
    /// Человекочитаемый итог («актуальная версия» / «доступна 0.12.0» / ошибка).
    pub text: String,
    /// Ссылка на релиз, если есть обновление (кнопка «Скачать»).
    pub url: String,
    pub is_newer: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LoginTab {
    Token,
    Email,
    Qr,
}

/// Стадии QR-логина (события discord://qr).
/// `Matrix.fingerprint` зарезервирован под проверку обновлений QR.
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
    /// Порядок рейла: отдельные серверы и папки (v0.12.1: раньше был
    /// плоский список — папки терялись, серверы сваливались в «колбасу»).
    pub rail: Vec<RailItem>,
    /// Папки рейла (индексуется RailItem::Folder).
    pub folders: Vec<GuildFolder>,
    /// Раскрытая папка (клик по папке разворачивает её столбиком серверов).
    pub open_folder: Option<usize>,
    /// Последний открытый канал каждой гильдии (автовыбор при клике
    /// по серверу — как в Tauri-фронтенде и официальном клиенте).
    pub last_channel_by_guild: HashMap<String, String>,
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
    /* ------- Редактирование/удаление/вложения (паритет с Tauri) ------- */
    /// Активное редактирование: (id сообщения, исходный текст).
    pub edit_target: Option<(String, String)>,
    /// Идёт загрузка вложений (кнопка 📎 заблокирована).
    pub uploading: bool,
    /* --------------------------- Стримы/камера --------------------------- */
    /// Все живые трансляции: stream_key → инфо (свои и чужие).
    pub streams: HashMap<String, StreamUi>,
    /// Ключ СВОЕЙ активной трансляции (владелец = self).
    pub my_stream_key: Option<String>,
    /// Камера включена (op 4 self_video). Превью — окно preview.rs.
    pub camera_on: bool,
    /// Лупбек микрофона включён (UI-зеркало флагов сессий).
    pub loopback_on: bool,
    /* ----------------------- Поведение (бывшие плагины) ----------------------- */
    /// Silent Typing: не отправлять индикатор «печатает…».
    pub suppress_typing: bool,
    /// Message Logger: показывать удалённые сообщения в баннере.
    pub log_deleted: bool,
    /* --------------------------- Модерация --------------------------- */
    /// Подтверждение кика/бана: (user_id, действие) — второй клик исполняет.
    pub confirm_mod: Option<(String, ModAction)>,
    /* --------------------------- Настройки --------------------------- */
    pub settings_open: bool,
    pub settings_section: SettingsSection,
    pub update_status: UpdateStatus,
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
            rail: Vec::new(),
            folders: Vec::new(),
            open_folder: None,
            last_channel_by_guild: HashMap::new(),
            reply_to: None,
            send_error: None,
            voice_connected: HashMap::new(),
            speaking: HashSet::new(),
            mic_level: 0.0,
            banner: None,
            scroll_bottom_pending: true,
            edit_target: None,
            uploading: false,
            streams: HashMap::new(),
            my_stream_key: None,
            camera_on: false,
            loopback_on: false,
            suppress_typing: false,
            log_deleted: false,
            confirm_mod: None,
            settings_open: false,
            settings_section: SettingsSection::Voice,
            update_status: UpdateStatus {
                checking: false,
                text: String::new(),
                url: String::new(),
                is_newer: false,
            },
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
    /// Локальная конфигурация (токен сессии, dev-режим) — персистентная.
    pub config: crate::config::Config,
    /// Мигание курсора (тикер подключим в следующей итерации).
    #[allow(dead_code)]
    pub blink_on: bool,
    /// Скролл ленты чата (автоскролл вниз при новых сообщениях).
    pub chat_scroll: gpui::ScrollHandle,
}

impl AppModel {
    pub fn new(
        core: Arc<AppState>,
        ui_tx: EventTx,
        config: crate::config::Config,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
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
            config,
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
            "discord://voice" => self.apply_voice(&payload, cx),
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
            "ui://upload-done" => {
                if let Screen::Main(m) = &mut self.screen {
                    m.uploading = false;
                }
                cx.notify();
            }
            "ui://upload-picked" => {
                // Файлы выбраны: текст композера уже уехал в multipart — чистим.
                if let Screen::Main(m) = &mut self.screen {
                    m.composer.update(cx, |f, cx| f.clear(cx));
                }
                cx.notify();
            }
            "ui://dm-created" => {
                let cid = payload
                    .get("channel_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !cid.is_empty() {
                    if let Screen::Main(m) = &mut self.screen {
                        m.selected_guild = None;
                    }
                    crate::actions::select_channel(self, &cid, cx);
                }
            }
            "ui://banner" => {
                if let Some(e) = payload.get("text").and_then(|v| v.as_str()) {
                    if let Screen::Main(m) = &mut self.screen {
                        m.banner = Some(e.to_string());
                    }
                }
                cx.notify();
            }
            // Окно превью закрыто/остановлено (кнопка, нативный стоп,
            // отмена выбора источника) — гасим соответствующий сигналинг.
            "ui://preview-stopped" => {
                let kind = payload.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                match kind {
                    "screen" => crate::actions::stop_my_stream(self, cx),
                    "camera" => crate::actions::camera_set(self, false, cx),
                    _ => {}
                }
            }
            "ui://update-checked" => {
                let text = payload
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = payload
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_newer = payload
                    .get("is_newer")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if let Screen::Main(m) = &mut self.screen {
                    m.update_status = UpdateStatus {
                        checking: false,
                        text,
                        url,
                        is_newer,
                    };
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

    /// rail_order + folders: раскладка рейла серверов пользователя.
    /// Полный порт раскладчика из Tauri-фронтенда: порядок interleaving
    /// сохраняется, папки живут в `folders` и рендерятся столбиком
    /// (раньше папки выбрасывались — серверы из них пропадали из рейла).
    fn apply_guild_layout(&mut self, payload: &Value) {
        let mut folders: Vec<GuildFolder> = Vec::new();
        if let Some(arr) = payload.get("folders").and_then(|v| v.as_array()) {
            for f in arr {
                let Some(ids) = f.get("guild_ids").and_then(|v| v.as_array()) else {
                    continue;
                };
                let guild_ids: Vec<String> = ids
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                if guild_ids.is_empty() {
                    continue;
                }
                folders.push(GuildFolder {
                    name: f.get("name").and_then(|v| v.as_str()).map(String::from),
                    color: f.get("color").and_then(|v| v.as_i64()),
                    guild_ids,
                });
            }
        }
        // rail_order: [{type:"guild", id} | {type:"folder", index}] —
        // точный порядок рейла. Хвостовые события от кэша ядра: GUILD_CREATE
        // дописывает только серверы, которых нет ни в одном элементе.
        let mut rail: Vec<RailItem> = Vec::new();
        if let Some(arr) = payload.get("rail_order").and_then(|v| v.as_array()) {
            for item in arr {
                match item.get("type").and_then(|v| v.as_str()) {
                    Some("guild") => {
                        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                            rail.push(RailItem::Guild(id.to_string()));
                        }
                    }
                    Some("folder") => {
                        let idx = item.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        if idx < folders.len() {
                            rail.push(RailItem::Folder(idx));
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Screen::Main(m) = &mut self.screen {
            if !folders.is_empty() || !rail.is_empty() {
                m.folders = folders;
                m.rail = rail;
                // Раскрытая папка могла исчезнуть из раскладки.
                if let Some(open) = m.open_folder {
                    if open >= m.folders.len() {
                        m.open_folder = None;
                    }
                }
            }
        }
    }

    fn apply_voice(&mut self, payload: &Value, cx: &mut gpui::Context<Self>) {
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
                        // Выход из войса убивает и нашу трансляцию/камеру
                        // (медиа-сервер завершит их сам — синхронизируем UI).
                        m.my_stream_key = None;
                        m.camera_on = false;
                        m.streams.retain(|_, s| s.guild_id.as_deref() != Some(gid));
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
                    // Индикаторы говорящих — перерисовываем.
                    cx.notify();
                }
                "mic_level" => {
                    // Телеметрия 20 Гц: в настройках «Голос» по ней живёт
                    // индикатор уровня — перерисовываем (лёгкий div-рендер).
                    m.mic_level = data
                        .get("level")
                        .and_then(|v| v.as_f64())
                        .map(|v| v as f32)
                        .unwrap_or(0.0);
                    cx.notify();
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
                    // Message Logger (нативный порт плагина): до удаления
                    // помним текст и автора — показываем в баннере.
                    if m.log_deleted {
                        if let Some(msg) = m
                            .messages
                            .get(ch)
                            .and_then(|list| list.iter().find(|x| x.id == id).cloned())
                        {
                            let author = msg
                                .author
                                .as_ref()
                                .map(|a| {
                                    a.global_name
                                        .clone()
                                        .or_else(|| Some(a.username.clone()))
                                        .unwrap_or_else(|| "?".into())
                                })
                                .unwrap_or_else(|| "?".into());
                            let preview: String = msg.content.chars().take(80).collect();
                            if !preview.is_empty() {
                                m.banner = Some(format!("🗑 {author} удалил: {preview}"));
                            }
                        }
                    }
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
                // Новая гильдия вне раскладки — в конец рейла (если её там ещё нет:
                // серверы из папок не дублируем отдельной плиткой).
                if let Some(gid) = d.get("id").and_then(|v| v.as_str()) {
                    if let Screen::Main(m) = &mut self.screen {
                        let known = m.rail.iter().any(|it| match it {
                            RailItem::Guild(id) => id == gid,
                            RailItem::Folder(i) => m
                                .folders
                                .get(*i)
                                .is_some_and(|f| f.guild_ids.iter().any(|x| x == gid)),
                        });
                        if !known {
                            m.rail.push(RailItem::Guild(gid.to_string()));
                        }
                    }
                }
            }
            // Go Live: op 18 создаёт трансляцию, сервер рассылает STREAM_CREATE
            // с stream_key (ядро уже распарсило guild/channel/user в поля d).
            // STREAM_UPDATE несёт paused/viewer_count, STREAM_DELETE — конец.
            "STREAM_CREATE" => {
                if let Screen::Main(m) = &mut self.screen {
                    if let Some(key) = d.get("stream_key").and_then(|v| v.as_str()) {
                        let me = self.core.self_id();
                        let user_id = d
                            .get("user_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let info = StreamUi {
                            stream_key: key.to_string(),
                            user_id: user_id.clone(),
                            guild_id: d
                                .get("guild_id")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            channel_id: d
                                .get("channel_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            paused: false,
                            viewers: 0,
                        };
                        if me.as_deref() == Some(user_id.as_str()) {
                            m.my_stream_key = Some(key.to_string());
                        }
                        m.streams.insert(key.to_string(), info);
                    }
                }
            }
            "STREAM_UPDATE" => {
                if let Screen::Main(m) = &mut self.screen {
                    if let Some(key) = d.get("stream_key").and_then(|v| v.as_str()) {
                        if let Some(info) = m.streams.get_mut(key) {
                            info.paused = d
                                .get("paused")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(info.paused);
                            info.viewers = d
                                .get("viewer_count")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(info.viewers);
                        }
                    }
                }
            }
            "STREAM_DELETE" => {
                if let Screen::Main(m) = &mut self.screen {
                    if let Some(key) = d.get("stream_key").and_then(|v| v.as_str()) {
                        if m.my_stream_key.as_deref() == Some(key) {
                            m.my_stream_key = None;
                        }
                        m.streams.remove(key);
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
            // Мёртвая сессия (401): сохранённый токен больше не пробуем —
            // иначе автологин будет долбить REST по кругу.
            if err.contains("401") {
                self.config.clear_token();
            }
            cx.notify();
            return;
        }
        // Успех: session::connect вернул профиль — сохраняем токен
        // (автологин при следующем запуске) и переключаемся на главный.
        if let Some(token) = self
            .core
            .token
            .read()
            .unwrap()
            .clone()
            .filter(|t| !t.is_empty())
        {
            self.config.set_token(&token);
        }
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
