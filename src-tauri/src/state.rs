//! Central application state: the protocol cache (what the official client
//! keeps in IndexedDB, we keep in a plain `RwLock`-guarded HashMap) plus
//! plumbing handles for the gateway task and voice sessions.

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::rest::models::{Channel, Member, Presence, Role, User, VoiceState};

/// Gateway intents we request on connect.
/// GUILDS | GUILD_MEMBERS* | GUILD_VOICE_STATES | GUILD_PRESENCES* |
/// GUILD_MESSAGES | GUILD_MESSAGE_TYPING | DIRECT_MESSAGES |
/// DIRECT_MESSAGE_TYPING | MESSAGE_CONTENT*   (* = privileged)
pub const INTENTS_FULL: u32 = (1 << 0)
    | (1 << 1)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 10)
    | (1 << 12)
    | (1 << 13)
    | (1 << 15);

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

pub struct AppState {
    pub app: tauri::AppHandle,
    pub cache: RwLock<Cache>,
    pub token: RwLock<Option<String>>,
    /// Active intents bitfield (may degrade after close code 4014).
    pub intents: AtomicU32,
    pub gateway_url: RwLock<String>,
    pub gateway_tx: RwLock<Option<mpsc::Sender<GatewayCommand>>>,
    pub gateway_task: tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub shutdown: RwLock<CancellationToken>,
    /// guild_id → partially assembled voice handshake.
    pub voice_handshake: Mutex<HashMap<String, VoiceHandshake>>,
    /// guild_id → running voice session.
    pub voice_tasks: tokio::sync::Mutex<
        HashMap<String, (tauri::async_runtime::JoinHandle<()>, CancellationToken)>,
    >,
    pub voice_notify: tokio::sync::Notify,
}

impl AppState {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            cache: RwLock::new(Cache::default()),
            token: RwLock::new(None),
            intents: AtomicU32::new(INTENTS_FULL),
            gateway_url: RwLock::new("wss://gateway.discord.gg".to_string()),
            gateway_tx: RwLock::new(None),
            gateway_task: tokio::sync::Mutex::new(None),
            shutdown: RwLock::new(CancellationToken::new()),
            voice_handshake: Mutex::new(HashMap::new()),
            voice_tasks: tokio::sync::Mutex::new(HashMap::new()),
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
