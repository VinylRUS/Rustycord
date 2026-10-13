//! Central application state: the protocol cache (what the official client
//! keeps in IndexedDB, we keep in a plain `RwLock`-guarded HashMap) plus
//! plumbing handles for the gateway task and voice sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::rest::models::{Channel, Member, Presence, Role, User, VoiceState};
use crate::voice::{VoiceControls, VoiceWsCmd};

/// Интенты пользовательской сессии: юзер-аккаунту доступны все биты без
/// аппрува в Developer Portal (в отличие от ботов). Включаем всё вплоть до
/// GUILD_SCHEDULED_EVENTS — как это делает официальный клиент.
///   0..14 — GUILDS … DIRECT_MESSAGE_TYPING
///   15    — MESSAGE_CONTENT (для ботов privileged, для юзера — обычный)
///   16    — GUILD_SCHEDULED_EVENTS
pub const INTENTS_USER: u32 = (1 << 0)
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
    /// op 8 — request the full member list of a guild (chunked reply).
    RequestMembers { guild_id: String },
    /// op 4 — join/leave a voice channel or change self mute/deaf.
    VoiceStateUpdate {
        guild_id: String,
        channel_id: Option<String>,
        self_mute: bool,
        self_deaf: bool,
    },
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
    pub channels: Vec<Channel>,
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
}

impl Cache {
    pub fn reset(&mut self) {
        self.user = None;
        self.guilds.clear();
        self.dms.clear();
    }
}

/// Voice handshake pieces assembled from two independent gateway events:
/// VOICE_STATE_UPDATE (session_id, for our own user) and
/// VOICE_SERVER_UPDATE (token + endpoint). Once both arrive for a guild,
/// the voice coordinator can dial the media server.
#[derive(Debug, Clone, Default)]
pub struct VoiceHandshake {
    pub guild_id: String,
    pub user_id: String,
    pub session_id: Option<String>,
    pub token: Option<String>,
    pub endpoint: Option<String>,
}

impl VoiceHandshake {
    pub fn complete(&self) -> bool {
        self.session_id.is_some() && self.token.is_some() && self.endpoint.is_some()
    }
}

/// Живая голосовая сессия: события эхо op4 (events.rs) обновляют мьют-флаги
/// через `controls`, а аудиоотправитель говорит сигналингу op 5 SPEAKING
/// через `ws_cmd`.
#[derive(Debug)]
pub struct VoiceSessionHandle {
    pub controls: Arc<Mutex<VoiceControls>>,
    /// Не читается намеренно: этот клон держит канал открытым всё время
    /// жизни сессии — иначе ранний выход отправителя (нет микрофона) закрыл
    /// бы WS-цикл через recv() → None. Читает ws.rs, дропает run_voice.
    #[allow(dead_code)]
    pub ws_cmd: mpsc::UnboundedSender<VoiceWsCmd>,
}

pub struct AppState {
    pub app: tauri::AppHandle,
    pub cache: RwLock<Cache>,
    pub token: RwLock<Option<String>>,
    /// Active intents bitfield (may degrade after close code 4014).
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
}

impl AppState {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            cache: RwLock::new(Cache::default()),
            token: RwLock::new(None),
            intents: AtomicU32::new(INTENTS_USER),
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
