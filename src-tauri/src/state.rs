//! Central application state: the protocol cache (what the official client
//! keeps in IndexedDB, we keep in a plain `RwLock`-guarded HashMap) plus
//! plumbing handles for the gateway task and voice sessions.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::rest::models::{Channel, Member, Presence, Relationship, Role, User, VoiceState};
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
    /// op 14 — lazy member list (user sessions): список участников канала
    /// диапазонами, ответ приезжает теми же GUILD_MEMBERS_CHUNK.
    SubscribeMemberList {
        guild_id: String,
        /// (channel_id, ranges) — обычно [[0, 99]] на каждый канал.
        channels: Vec<(String, Vec<u32>)>,
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
/// (events.rs), `driver` — клон канала управления движком (mute/leave).
#[derive(Clone)]
pub struct VoiceSessionHandle {
    pub controls: Arc<Mutex<VoiceControls>>,
    pub driver: Option<songbird::Driver>,
    /// Роль песни «микрофон»: держим, чтобы трек не завершался сам.
    #[allow(dead_code)]
    pub mic_track: Option<songbird::tracks::TrackHandle>,
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
