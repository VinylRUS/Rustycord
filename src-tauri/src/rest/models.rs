//! Discord API v10 wire models.
//!
//! Design note: field names stay snake_case (exactly what Discord sends) so
//! payloads can be forwarded to the UI and back without any remapping —
//! keeping the "protocol shapes" visible is the point of this project.
//! `Option` fields are natively optional in serde; plain fields carry
//! `#[serde(default)]` because Discord loves sending partial objects
//! (MESSAGE_UPDATE can be as small as `{id, channel_id}`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub accent_color: Option<i64>,
    #[serde(default)]
    pub bot: bool,
}

/// `/users/@me` returns the same shape — bot identity included.
pub type CurrentUser = User;

/// Relationship entry from the user READY payload / RELATIONSHIP_ADD.
/// `kind` follows the protocol numbering:
/// 1 = friend, 2 = blocked, 3 = pending incoming, 4 = pending outgoing.
/// User accounts only — bots don't have relationships.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u8,
    pub user: User,
    #[serde(default)]
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub permissions: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub mentionable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub joined_at: Option<String>,
    #[serde(default)]
    pub premium_since: Option<String>,
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub communication_disabled_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub nsfw: Option<bool>,
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub rate_limit_per_user: Option<u32>,
    #[serde(default)]
    pub recipients: Option<Vec<User>>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub roles: Vec<Role>,
    // Present only in GUILD_CREATE (gateway), never in REST responses:
    #[serde(default)]
    pub channels: Option<Vec<Channel>>,
    #[serde(default)]
    pub members: Option<Vec<Member>>,
    #[serde(default)]
    pub presences: Option<Vec<Presence>>,
    #[serde(default)]
    pub voice_states: Option<Vec<VoiceState>>,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub large: Option<bool>,
    #[serde(default)]
    pub unavailable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emoji {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub me: bool,
    pub emoji: Emoji,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub member: Option<Member>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default)]
    pub tts: bool,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub mentions: Vec<User>,
    #[serde(default)]
    pub mention_roles: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub embeds: Vec<serde_json::Value>,
    #[serde(default)]
    pub reactions: Option<Vec<Reaction>>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(rename = "type", default)]
    pub kind: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: u8,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
}

/// Partial user embedded in PRESENCE_UPDATE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUser {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    pub user: PresenceUser,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub activities: Vec<Activity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceState {
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    pub user_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub self_deaf: bool,
    #[serde(default)]
    pub self_mute: bool,
    #[serde(default)]
    pub self_video: bool,
    #[serde(default)]
    pub suppress: bool,
}
