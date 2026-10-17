//! Central application state: the protocol cache (what the official client
//! keeps in IndexedDB, we keep in a plain `RwLock`-guarded HashMap) plus
//! plumbing handles for the gateway task and voice sessions.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::rest::models::{Channel, Member, Presence, Relationship, Role, User, VoiceState};
use crate::voice::dsp::{DspConfig, DspMetrics};
use crate::voice::VoiceControls;

/// Интенты для БОТ-сессии (юзер-аккаунты их не шлют — см. `is_bot`).
///   0..14 — GUILDS … DIRECT_MESSAGE_TYPING
///   15    — MESSAGE_CONTENT (privileged для ботов)
///   16    — GUILD_SCHEDULED_EVENTS
pub const INTENTS_BOT: u32 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 10)
    | (1 << 11)
    | (1 << 12)
    | (1 << 13)
    | (1 << 14)
    | (1 << 15)
    | (1 << 16);

/// Graceful degradation when the portal has privileged intents disabled:
/// we drop the three privileged bits and keep basic chat working.
pub const INTENTS_FALLBACK: u32 =
    (1 << 0) | (1 << 7) | (1 << 9) | (1 << 10) | (1 << 12) | (1 << 13);

/// Messages other tasks can push into the gateway connection.
#[derive(Clone, Debug)]
pub enum GatewayCommand {
    /// Raw JSON frame (escape hatch / protocol experiments).
    SendText(String),
    /// op 8 — request the full member list of a guild (chunked reply) — bots.
    RequestMembers { guild_id: String },
    /// op 37 — bulk guild subscribe (user sessions: presences/typing/threads
    /// для всех гильдий сразу — так делает официальный клиент после READY).
    BulkGuildSubscribe { guild_ids: Vec<String> },
    /// op 37 — подписка на ленивый список участников ОДНОГО канала
    /// (member-диапазоны едут внутри подписки — как в официальном клиенте:
    /// subscribeChannel кладёт {channels:{cid:ranges}} в op 37).
    /// ВАЖНО: официальный клиент держит LRU максимум из 5 каналов на
    /// гильдию (ChannelsStore `new LRU({max: 5})` в бандле discord.com) —
    /// живой шлюз закрывает соединение 4002 «Error while decoding payload»,
    /// если в одном op 37 прислать больше (v0.9.1 слал 25 и ловил 4002
    /// на каждый клик по серверу). Поэтому здесь РОВНО ОДИН канал —
    /// текущий; `None` → псевдоканал "0" (общий список гильдии, так
    /// делает официальный клиент для member sidebar без канала).
    /// Диапазоны ([[0,99],[100,199],…) считает фронтенд по скроллу —
    /// ровно как onScroll → GUILD_SUBSCRIPTIONS_CHANNEL у официала:
    /// всегда включает [0,99], дальше сотнями до видимой зоны + overscan.
    /// Ответ: GUILD_MEMBER_LIST_UPDATE (SYNC/INSERT/UPDATE/DELETE ops).
    SubscribeMemberList {
        guild_id: String,
        /// Канал, чей список участников показать (None → список гильдии).
        channel_id: Option<String>,
        /// Диапазоны строк [начало, конец] (включительно), по 100 строк.
        ranges: Vec<(u64, u64)>,
    },
    /// op 4 — join/leave a voice channel or change self mute/deaf.
    /// `guild_id: None` — звонок в ЛС/группе (протокол DM-звонков: op4 уходит
    /// с guild_id:null, а voice-сервер идентифицирует сессию по channel_id).
    VoiceStateUpdate {
        guild_id: Option<String>,
        channel_id: Option<String>,
        self_mute: bool,
        self_deaf: bool,
        /// self_video — «камера включена» (как в официальном клиенте).
        self_video: bool,
    },
    /// op 18 STREAM_CREATE — начать трансляцию (screen/camera) в голосовом
    /// канале. Точный формат из discord-client-rs (create_op_18).
    StreamCreate {
        /// "screen" | "camera" — тип трансляции.
        stream_type: String,
        guild_id: String,
        channel_id: String,
    },
    /// op 20 STREAM_DELETE — прекратить трансляцию по ключу
    /// (stream_key прилетает в dispatch STREAM_CREATE).
    StreamDelete { stream_key: String },
    /// op 22 STREAM_SET_PAUSED — пауза/снятие трансляции.
    StreamSetPaused { stream_key: String, paused: bool },
}

/// Строка ленивого списка участников (GUILD_MEMBER_LIST_UPDATE).
/// Сервер присылает УЖЕ собранный сайдбар: заголовки групп и участников
/// вперемешку, в готовом порядке — рендерим как есть (1:1 с официалом).
/// Размен размеров вариантов сознателен: строки живут в Vec и уезжают в
/// JSON-сериализацию, боксовать поля незачем.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemberListRow {
    /// Заголовок секции: «В сети», роль или «Оффлайн».
    Group { id: String, count: u64 },
    /// Участник (с presence внутри — статусы едут сразу с карточкой).
    Member {
        user_id: String,
        username: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        global_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        nick: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        discriminator: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        avatar: Option<String>,
        #[serde(skip_serializing_if = "std::ops::Not::not", default)]
        bot: bool,
        /// online / idle / dnd / offline (offline-строки не приходят —
        /// сервер держит их за пределами диапазона, но поле на всякий случай).
        status: String,
        /// Активности (кастомный статус / игра) — для подписи под ником.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        activities: Vec<serde_json::Value>,
        #[serde(default)]
        roles: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hoisted_role: Option<String>,
    },
}

/// Трекер серверного id ленивого списка (урок v0.9.5: `d.id` в
/// GUILD_MEMBER_LIST_UPDATE — НЕ id канала). Идентификатор списка
/// вычисляется сервером из permission overwrites канала (list_id
/// официального клиента): у каналов без @everyone-deny на VIEW_CHANNEL
/// (подавляющее большинство) список общий — `"everyone"`, у приватных —
/// вычисленное значение. Сервер держит подписки до 5 каналов на гильдию
/// и присылает события ВСЕХ активных списков — трекер отличает «наш»
/// список от хвостов прошлых подписок.
///
/// Семантика состояний:
/// * `active = Some(id)` — список известен (предсеян или выучен из SYNC):
///   события чужих id — хвосты, мимо. Исключение — SYNC с чужим id
///   (escape-hatch): серверный снимок авторитетнее нашего предсеяния,
///   перелокируемся (роль-deny каналы выглядят «обычными», а список у
///   них вычисленный — ошибка предсеяния обязана самолечиться, иначе
///   сайдбар пуст навсегда).
/// * `active = None, expect_private` — канал приватный (@everyone-deny):
///   ждём только SYNC (id знает только сервер), хвосты — мимо.
/// * `active = None, !expect_private` — канал неизвестен кэшу: принимаем
///   первое пришедшее событие любого id (самокорректируется).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ActiveListTracker {
    /// Серверный id активного списка (None — ждём первый SYNC).
    pub active: Option<String>,
    /// Подписанный канал приватный (@everyone deny VIEW_CHANNEL) — его
    /// список не `everyone`.
    pub expect_private: bool,
}

impl ActiveListTracker {
    /// Принять событие с серверным id `list_id` как текущий список?
    /// `has_sync` — есть ли в событии SYNC-op (авторитетный снимок).
    pub fn accept(&mut self, list_id: &str, has_sync: bool) -> bool {
        if let Some(active) = &self.active {
            if active == list_id {
                return true;
            }
            // SYNC с чужим id: предсеяние было ошибочным (роль-deny) или
            // список сменился — сервер знает лучше. Перелокируемся.
            if has_sync {
                self.active = Some(list_id.to_string());
                self.expect_private = false;
                return true;
            }
            return false; // хвост прежней подписки (LRU-5)
        }
        if self.expect_private && !has_sync {
            return false; // приватный список: ждём его SYNC, хвосты — мимо
        }
        self.active = Some(list_id.to_string());
        true
    }
}

/// Целевой список для подписки канала. Цепочка точности (каждое звено
/// точнее следующего):
/// 1. Выученный id канала (channel_lists из прошлого SYNC) — точный.
/// 2. Выученный id списка ГИЛЬДИИ (guild_list из последнего принятого
///    события) — серверы со скрытым списком участников вычисляют ОДИН
///    id для всех каналов; без этого предсеяния смена канала = пустой
///    сайдбар навсегда (пара уже подписана, SYNC повторно не приедет —
///    регресс v0.9.7, лог 22:37:44).
/// 3. «everyone» для доказуемо обычного канала (@everyone не запрещён
///    VIEW_CHANNEL).
/// 4. None — id знает только сервер (приватный/неизвестный канал):
///    трекер примет первое событие.
pub fn target_list_for_switch(
    known_list: Option<&str>,
    channel_private: Option<bool>,
    guild_list: Option<&str>,
) -> Option<String> {
    match known_list {
        Some(t) => Some(t.to_string()),
        None => match guild_list {
            Some(g) => Some(g.to_string()),
            None => {
                // Обычный канал — список 'everyone' (предсеем: SYNC не обязан
                // прийти повторно, если 'everyone'-диапазоны уже подписаны).
                if channel_private == Some(false) {
                    Some("everyone".to_string())
                } else {
                    None
                }
            }
        },
    }
}

/// Политика трекера для подписки канала (см. target_list_for_switch).
pub fn tracker_for_switch(
    known_list: Option<&str>,
    channel_private: Option<bool>,
    guild_list: Option<&str>,
) -> ActiveListTracker {
    match target_list_for_switch(known_list, channel_private, guild_list) {
        Some(t) => ActiveListTracker {
            expect_private: t != "everyone",
            active: Some(t),
        },
        None if channel_private == Some(true) => ActiveListTracker {
            active: None,
            expect_private: true,
        },
        None => ActiveListTracker::default(),
    }
}

/// Снимок строк одного списка (для мини-кэша per-list).
#[derive(Debug, Clone, Default)]
pub struct ListSnapshot {
    pub rows: Vec<MemberListRow>,
    pub member_count: u64,
    pub online_count: u64,
}

/// Снимок ленивого списка участников гильдии (по текущему каналу).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemberListState {
    /// Канал, на который подписаны ("0" — общий список гильдии).
    pub channel_id: String,
    pub member_count: u64,
    pub online_count: u64,
    /// Подписанные диапазоны строк — для replay после переподключения
    /// (CONNECTION_OPEN/RESUMED → flush, как GuildSubscriptionsStore).
    #[serde(skip)]
    pub ranges: Vec<(u64, u64)>,
    pub rows: Vec<MemberListRow>,
    /// Серверный id активного списка ('everyone'/вычисленный) — матчит
    /// GUILD_MEMBER_LIST_UPDATE.d.id (см. ActiveListTracker).
    #[serde(skip)]
    pub tracker: ActiveListTracker,
    /// Какому списку принадлежат текущие rows (provenance): protects от
    /// чужих строк и управляет stash/restore.
    #[serde(skip)]
    pub rows_list: Option<String>,
    /// Выученные id списков по каналам (канал → list id первого SYNC):
    /// повторные визиты канала предсеиваются мгновенно, без ожидания SYNC.
    #[serde(skip)]
    pub channel_lists: HashMap<String, String>,
    /// Выученный id списка ГИЛЬДИИ (последний принятый сервером
    /// подтверждённый id): серверы со скрытым списком вычисляют один id
    /// для всех каналов — предсеивание при смене канала, когда
    /// channel_lists ещё пуст. Обновляется ТОЛЬКО серверными событиями
    /// (не предсеяниями трекера — иначе гадание стало бы «фактом»).
    #[serde(skip)]
    pub guild_list: Option<String>,
    /// Мини-кэш строк по list id — как GuildMemberListStore официала:
    /// сервер шлёт SYNC только для НОВЫХ пар (список, диапазон), поэтому
    /// при переключении каналов строки прячу/восстанавливаю, а не качаю
    /// заново (стирание = пустой сайдбар — регресс v0.9.6).
    #[serde(skip)]
    pub list_cache: HashMap<String, ListSnapshot>,
}

impl MemberListState {
    /// Переключить данные на список `target` (None — неизвестен: обнулить).
    /// Текущие строки прячутся в кэш (для возврата), целевые — из кэша.
    pub fn switch_to_list(&mut self, target: Option<&str>) {
        // Тот же список — ничего не делаем (главный случай: обычные
        // каналы сервера делят один 'everyone', SYNC повторно не приедет).
        if self.rows_list.as_deref() == target && target.is_some() {
            return;
        }
        // Спрятать текущие строки под их списком.
        if let Some(cur) = self.rows_list.clone() {
            if !self.rows.is_empty() {
                self.list_cache.insert(
                    cur,
                    ListSnapshot {
                        rows: std::mem::take(&mut self.rows),
                        member_count: self.member_count,
                        online_count: self.online_count,
                    },
                );
                // Сервер держит ≤5 списков на гильдию — кэшу хватит 6.
                while self.list_cache.len() > 6 {
                    if let Some(oldest) = self.list_cache.keys().next().cloned() {
                        self.list_cache.remove(&oldest);
                    }
                }
            }
        }
        self.member_count = 0;
        self.online_count = 0;
        // Восстановить целевые (если кэшированы).
        if let Some(snap) = target.and_then(|t| self.list_cache.get(t).cloned()) {
            self.rows = snap.rows;
            self.member_count = snap.member_count;
            self.online_count = snap.online_count;
        } else {
            self.rows.clear();
        }
        self.rows_list = target.map(String::from);
    }
}

/// Per-guild cached state, kept fresh from gateway dispatches.
#[derive(Debug, Clone)]
pub struct GuildState {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub owner_id: String,
    pub roles: Vec<Role>,
    /// Кастомные эмодзи сервера (для эмодзи-пикера реакций).
    pub emojis: Vec<crate::rest::models::Emoji>,
    pub channels: Vec<Channel>,
    /// Активные треды (type 11/12) — отдельная коллекция: в сайдбаре они
    /// рендерятся под родительскими каналами, в REST — свой эндпоинт.
    pub threads: Vec<Channel>,
    pub members: HashMap<String, Member>,
    pub presences: HashMap<String, Presence>,
    pub voice_states: Vec<VoiceState>,
    pub member_count: u64,
}

/// The whole client-side cache. Locks are held only for short, non-async scopes.
#[derive(Debug, Default)]
pub struct Cache {
    pub user: Option<User>,
    pub guilds: HashMap<String, GuildState>,
    pub dms: Vec<Channel>,
    /// User-account friends/pending/blocked (user_id → relationship).
    /// Bots never populate this — relationships are a user-session concept.
    pub relationships: HashMap<String, Relationship>,
    /// Presences of friends (PRESENCE_UPDATE without guild_id + READY
    /// `presences` array) — guild-member presences live in GuildState.
    pub friend_presences: HashMap<String, Presence>,
}

impl Cache {
    pub fn reset(&mut self) {
        self.user = None;
        self.guilds.clear();
        self.dms.clear();
        self.relationships.clear();
        self.friend_presences.clear();
    }
}

/// Voice handshake pieces assembled from two independent gateway events:
/// VOICE_STATE_UPDATE (session_id, channel_id, for our own user) and
/// VOICE_SERVER_UPDATE (token + endpoint). Once both arrive for a guild,
/// the voice coordinator can dial the media server.
#[derive(Debug, Clone, Default)]
pub struct VoiceHandshake {
    /// Ключ сессии: guild_id — для серверного войса, channel_id — для
    /// звонков в ЛС/группе (голосовой сервер идентифицирует DM-сессию
    /// по каналу, он же уходит в songbird как server_id).
    pub guild_id: String,
    pub user_id: String,
    pub session_id: Option<String>,
    pub token: Option<String>,
    pub endpoint: Option<String>,
    /// Канал, в который ведём рукопожатие (из эха op4) — нужен songbird'у
    /// для ConnectionInfo, VOICE_SERVER_UPDATE его не несёт.
    pub channel_id: Option<String>,
    /// Звонок в ЛС/группе: VOICE_STATE/VOICE_SERVER_UPDATE приходят с
    /// guild_id:null — рукопожатие сшивается по этому флагу + session_id.
    pub is_dm: bool,
}

impl VoiceHandshake {
    pub fn complete(&self) -> bool {
        self.session_id.is_some() && self.token.is_some() && self.endpoint.is_some()
    }
}

/// Живая голосовая сессия на songbird: `controls` обновляется эхом op4
/// (events.rs), `driver` — общий экземпляр движка (mute).
///
/// ВАЖНО: `Driver` в songbird при **дропе** отправляет ядру `Poison` и убивает
/// всю сессию (см. `impl Drop for Driver`). Клонировать его нельзя — клоны
/// в обработчиках эха умирали в конце скоупа и отравляли ядро: трек
/// микрофона удалялся из микшера, чтения моста останавливались («звонок
/// падает при включении трансляции/деафена»). Поэтому расшариваем ОДИН
/// экземпляр через `Arc<Mutex<..>>`: дропается он только вместе с сессией —
/// ровно тогда, когда ядро и должно умереть.
#[derive(Clone)]
pub struct VoiceSessionHandle {
    pub controls: Arc<Mutex<VoiceControls>>,
    pub driver: Option<Arc<Mutex<songbird::Driver>>>,
    /// Роль песни «микрофон»: держим, чтобы трек не завершался сам.
    #[allow(dead_code)]
    pub mic_track: Option<songbird::tracks::TrackHandle>,
    /// Локальный тест микрофона (voice_loopback): захват → свои динамики
    /// без Discord. Флаг читает InTap (devices.rs) в cpal-колбеке ввода.
    pub loopback: Arc<AtomicBool>,
}

pub struct AppState {
    pub app: tauri::AppHandle,
    pub cache: RwLock<Cache>,
    pub token: RwLock<Option<String>>,
    /// Бот-аккаунт? Юзер-сессии шлют capabilities вместо intents и получают
    /// все события без фильтрации (как официальный клиент).
    pub is_bot: std::sync::atomic::AtomicBool,
    /// Active intents bitfield (бот-сессии; may degrade after close 4014).
    pub intents: AtomicU32,
    /// client_build_number для X-Super-Properties (тянем с discord.com).
    pub build_number: AtomicU64,
    pub gateway_url: RwLock<String>,
    pub gateway_tx: RwLock<Option<mpsc::Sender<GatewayCommand>>>,
    pub gateway_task: tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub shutdown: RwLock<CancellationToken>,
    /// QR remote-auth: таск + токен отмены (живёт только на экране логина).
    pub qr_task: tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub qr_cancel: RwLock<Option<CancellationToken>>,
    /// guild_id → partially assembled voice handshake.
    pub voice_handshake: Mutex<HashMap<String, VoiceHandshake>>,
    /// guild_id → running voice session.
    pub voice_tasks: tokio::sync::Mutex<
        HashMap<String, (tauri::async_runtime::JoinHandle<()>, CancellationToken)>,
    >,
    /// guild_id → controls for a live voice session (mute flags + WS commands).
    pub voice_sessions: Mutex<HashMap<String, VoiceSessionHandle>>,
    /// Last desired op4 flags (written by join_voice, read by a starting session).
    pub voice_desired: Mutex<HashMap<String, (bool, bool)>>,
    pub voice_notify: tokio::sync::Notify,
    /// Гильдии, на которые уже отправлены op 37 + op 14 (юзер-сессии).
    /// Дедуп: повторное открытие сервера не спамит gateway подписками.
    pub subscribed_guilds: Mutex<HashSet<String>>,
    /// Настройки голосового DSP (шумодав + AGC). Пишутся командой
    /// set_voice_dsp, читаются аудио-колбеком живой сессии на лету.
    pub voice_dsp_cfg: Arc<Mutex<DspConfig>>,
    /// Телеметрия DSP (уровни, усиление, VAD) — рисуется в настройках.
    pub voice_dsp_metrics: Arc<Mutex<DspMetrics>>,
    /// Открыт ли микрофон прямо сейчас (для честного «нет сессии» в UI).
    pub voice_dsp_active: Arc<AtomicBool>,
    /// Ленивые списки участников (guild_id → снимок по текущему каналу).
    /// Заполняется GUILD_MEMBER_LIST_UPDATE; channel_id хранится и после
    /// обрыва — после переподключения READY-хендлер переигрывает подписку
    /// (как CONNECTION_OPEN → flush в официальном клиенте).
    pub member_lists: Mutex<HashMap<String, MemberListState>>,
}

impl AppState {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            cache: RwLock::new(Cache::default()),
            token: RwLock::new(None),
            is_bot: std::sync::atomic::AtomicBool::new(false),
            intents: AtomicU32::new(INTENTS_BOT),
            build_number: AtomicU64::new(crate::superprops::FALLBACK_BUILD_NUMBER),
            gateway_url: RwLock::new("wss://gateway.discord.gg".to_string()),
            gateway_tx: RwLock::new(None),
            gateway_task: tokio::sync::Mutex::new(None),
            shutdown: RwLock::new(CancellationToken::new()),
            qr_task: tokio::sync::Mutex::new(None),
            qr_cancel: RwLock::new(None),
            voice_handshake: Mutex::new(HashMap::new()),
            voice_tasks: tokio::sync::Mutex::new(HashMap::new()),
            voice_sessions: Mutex::new(HashMap::new()),
            voice_desired: Mutex::new(HashMap::new()),
            voice_notify: tokio::sync::Notify::new(),
            subscribed_guilds: Mutex::new(HashSet::new()),
            voice_dsp_cfg: Arc::new(Mutex::new(DspConfig::default())),
            voice_dsp_metrics: Arc::new(Mutex::new(DspMetrics::default())),
            voice_dsp_active: Arc::new(AtomicBool::new(false)),
            member_lists: Mutex::new(HashMap::new()),
        }
    }

    /// Convenience for voice: the bot's own user id.
    pub fn self_id(&self) -> Option<String> {
        self.cache
            .read()
            .unwrap()
            .user
            .as_ref()
            .map(|u| u.id.clone())
    }

    /// Broadcast a status frame to the UI.
    pub fn emit_status(&self, stage: &str, detail: Option<String>) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "discord://status",
            serde_json::json!({ "stage": stage, "detail": detail }),
        );
    }
}

pub type SharedState = Arc<AppState>;
