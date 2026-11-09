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
    #[serde(default, deserialize_with = "de_permissions")]
    pub permissions: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub mentionable: bool,
}

/// Discord шлёт `permissions` строкой (значения бывают > 2^53), но в
/// отдельных payload'ах юзер-сессий встречается и число. Принимаем оба,
/// чтобы одна роль не роняла парсинг всей гильдии.
fn de_permissions<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    struct V;
    impl serde::de::Visitor<'_> for V {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("string or number")
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
    }
    d.deserialize_any(V)
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

/// Перезапись прав на канал (permission overwrite): `id` — роль или юзер
/// (`type`: 0 = роль, 1 = участник; для @everyone id = guild_id).
/// allow/deny — битовые маски строкой (значения бывают > 2^53).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Overwrite {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default, deserialize_with = "de_permissions")]
    pub allow: String,
    #[serde(default, deserialize_with = "de_permissions")]
    pub deny: String,
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
    /// Перезаписи прав канала — по ним UI скрывает каналы, недоступные
    /// роли пользователя (VIEW_CHANNEL). Присутствуют в GUILD_CREATE,
    /// CHANNEL_CREATE/UPDATE — REST-объекты каналов их тоже несут.
    #[serde(default)]
    pub permission_overwrites: Vec<Overwrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guild {
    pub id: String,
    /// Юзер-сессии шлют GUILD_CREATE БЕЗ верхнеуровневого name — имя
    /// живёт в properties.name (подтверждено живыми логами v0.7.1:
    /// «строгий разбор не прошёл: missing field `name`»). Поэтому поле
    /// опционально на уровне serde, а display-имя добирается через
    /// [`Guild::name_from_payload`] в обработчике gateway.
    #[serde(default)]
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
    /// Кастомные эмодзи сервера (GUILD_CREATE/GUILD_UPDATE для юзер-сессий).
    #[serde(default)]
    pub emojis: Option<Vec<Emoji>>,
    /// Активные треды гильдии (юзер-GUILD_CREATE несёт отдельный массив
    /// `threads` — треды идут отдельным массивом, не среди каналов).
    #[serde(default)]
    pub threads: Option<Vec<Channel>>,
}

impl Guild {
    /// Display-имя гильдии из сырого payload: верхнеуровневое `name`,
    /// а для юзер-сессий — `properties.name`. Возвращает None, если
    /// имени нет вовсе (ленивая гильдия) — вызывающий берёт из кэша.
    pub fn name_from_payload(v: &serde_json::Value) -> Option<String> {
        v.get("name")
            .and_then(|x| x.as_str())
            .or_else(|| {
                v.get("properties")
                    .and_then(|p| p.get("name"))
                    .and_then(|x| x.as_str())
            })
            .filter(|s| !s.is_empty())
            .map(String::from)
    }

    /// Иконка гильдии из сырого payload: верхнеуровневый `icon` или
    /// `properties.icon` (юзер-GUILD_CREATE прячет display-данные в
    /// properties — как name; скрин v0.9.5: активная гильдия с каналами
    /// рендерилась инициалами, потому что icon никто не смотрел там).
    pub fn icon_from_payload(v: &serde_json::Value) -> Option<String> {
        v.get("icon")
            .and_then(|x| x.as_str())
            .or_else(|| {
                v.get("properties")
                    .and_then(|p| p.get("icon"))
                    .and_then(|x| x.as_str())
            })
            .filter(|s| !s.is_empty())
            .map(String::from)
    }

    /// Salvage-разбор GUILD_CREATE из сырого JSON: оболочка гильдии
    /// вытягивается толерантно, а коллекции (каналы/роли/участники/
    /// пресенсы/войс-состояния/эмодзи) разбираются ПОЭЛЕМЕНТНО — битая
    /// запись выбрасывается с подсчётом, а не роняет всю гильдию.
    /// Возвращает None только если нет id (запись бесполезна в принципе).
    pub fn salvage_from_value(v: &serde_json::Value) -> Option<Guild> {
        let id = v.get("id")?.as_str()?.to_string();
        let name = Guild::name_from_payload(v).unwrap_or_else(|| "…".to_string());

        let (channels, ch_bad) = salvage_list::<Channel>(v.get("channels"));
        let (roles, role_bad) = salvage_list::<Role>(v.get("roles"));
        let (members, m_bad) = salvage_list::<Member>(v.get("members"));
        let (presences, p_bad) = salvage_list::<Presence>(v.get("presences"));
        let (voice_states, vs_bad) = salvage_list::<VoiceState>(v.get("voice_states"));
        let (emojis, e_bad) = salvage_list::<Emoji>(v.get("emojis"));
        let (threads, t_bad) = salvage_list::<Channel>(v.get("threads"));

        let bad = ch_bad + role_bad + m_bad + p_bad + vs_bad + e_bad + t_bad;
        if bad > 0 {
            let total = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_array())
                    .map(Vec::len)
                    .unwrap_or(0)
            };
            tracing::warn!(
                "gateway: salvage «{}»: каналов {}/{}, пропущено битых — ролей {}/{}, \
                 участников {}/{}, пресенсов {}/{}, войс {}/{}, эмодзи {}/{}, тредов {}/{}",
                name,
                channels.as_ref().map(Vec::len).unwrap_or(0),
                total("channels"),
                role_bad,
                total("roles"),
                m_bad,
                total("members"),
                p_bad,
                total("presences"),
                vs_bad,
                total("voice_states"),
                e_bad,
                total("emojis"),
                t_bad,
                total("threads"),
            );
        }

        Some(Guild {
            id,
            name,
            icon: v.get("icon").and_then(|x| x.as_str()).map(String::from),
            banner: v.get("banner").and_then(|x| x.as_str()).map(String::from),
            owner_id: v
                .get("owner_id")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            roles: roles.unwrap_or_default(),
            channels,
            members,
            presences,
            voice_states,
            member_count: v.get("member_count").and_then(|x| x.as_u64()),
            large: v.get("large").and_then(|x| x.as_bool()),
            unavailable: v.get("unavailable").and_then(|x| x.as_bool()),
            emojis,
            threads,
        })
    }
}

/// Поэлементный разбор массива из сырого JSON: битые элементы
/// пропускаются (с точной причиной — на уровне debug) и считаются.
fn salvage_list<T: serde::de::DeserializeOwned>(
    v: Option<&serde_json::Value>,
) -> (Option<Vec<T>>, usize) {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return (None, 0);
    };
    let mut out = Vec::with_capacity(arr.len());
    let mut bad = 0usize;
    for item in arr {
        match serde_json::from_value::<T>(item.clone()) {
            Ok(x) => out.push(x),
            Err(e) => {
                bad += 1;
                tracing::debug!("gateway: salvage пропустил битый элемент: {e}");
            }
        }
    }
    (Some(out), bad)
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Реалистичный GUILD_CREATE юзер-сессии (gateway v9, приходит после
    /// op 37-подписки): каналы всех типов, роли, участники, пресенсы
    /// с частичным user, войс-состояния, threads отдельным полем.
    fn realistic_guild_create() -> serde_json::Value {
        json!({
            "id": "918746238947562345",
            "name": "Тестовый сервер",
            "icon": "aabbccddeeff",
            "owner_id": "111111111111111111",
            "roles": [
                {"id": "918746238947562345", "name": "@everyone",
                 "permissions": "137438953471", "position": 0, "color": 0,
                 "hoist": false, "managed": false, "mentionable": false},
                {"id": "918746238947562346", "name": "Модератор",
                 "permissions": "1099511627775", "position": 1, "color": 15158332,
                 "hoist": true, "managed": false, "mentionable": true, "icon": null}
            ],
            "emojis": [
                {"id": "999999999999999991", "name": "pepe", "animated": false},
                {"id": "999999999999999992", "name": null, "animated": false}
            ],
            "features": ["COMMUNITY", "INVITE_SPLASH"],
            "premium_tier": 2,
            "member_count": 42,
            "large": false,
            "unavailable": false,
            "channels": [
                {"id": "100000000000000001", "type": 0, "name": "основной-чат",
                 "position": 0, "topic": "правила в закрепе",
                 "last_message_id": "100000000000000099", "rate_limit_per_user": 0,
                 "parent_id": "100000000000000010", "permission_overwrites": []},
                {"id": "100000000000000010", "type": 4, "name": "ТЕКСТ", "position": 0},
                {"id": "100000000000000020", "type": 2, "name": "Голосовой",
                 "position": 1, "bitrate": 64000, "user_limit": 0, "rtc_region": null},
                {"id": "100000000000000030", "type": 5, "name": "анонсы", "position": 2},
                {"id": "100000000000000040", "type": 13, "name": "сцена", "position": 3},
                {"id": "100000000000000050", "type": 15, "name": "форум", "position": 4}
            ],
            "threads": [
                {"id": "100000000000000060", "type": 11, "name": "ответвление",
                 "parent_id": "100000000000000001", "owner_id": "111111111111111111"}
            ],
            "members": [
                {"user": {"id": "111111111111111111", "username": "owner",
                          "discriminator": "0", "global_name": "Владелец",
                          "avatar": null, "bot": false},
                 "nick": null, "roles": [],
                 "joined_at": "2023-01-01T00:00:00.000000+00:00",
                 "deaf": false, "mute": false, "pending": false,
                 "communication_disabled_until": null},
                {"user": {"id": "222222222222222222", "username": "friend",
                          "discriminator": "0", "global_name": "Друг",
                          "avatar": "xx", "bot": true},
                 "nick": "Ник", "roles": ["918746238947562346"],
                 "joined_at": "2024-06-06T00:00:00.000000+00:00",
                 "premium_since": null, "deaf": false, "mute": false,
                 "pending": false}
            ],
            "presences": [
                {"user": {"id": "111111111111111111"}, "status": "online",
                 "activities": [], "client_status": {"web": "online"}},
                {"user": {"id": "222222222222222222", "username": "friend"},
                 "status": "idle",
                 "activities": [{"name": "Visual Studio Code", "type": 0,
                                 "created_at": 1728900000000u64}]}
            ],
            "voice_states": [
                {"guild_id": "918746238947562345",
                 "channel_id": "100000000000000020",
                 "user_id": "222222222222222222",
                 "session_id": "abcdef0123456789",
                 "deaf": false, "mute": false, "self_deaf": false,
                 "self_mute": true, "self_video": false, "suppress": false}
            ]
        })
    }

    #[test]
    fn guild_create_strict_parses() {
        let g: Guild = serde_json::from_value(realistic_guild_create())
            .expect("строгий разбор реалистичного payload");
        assert_eq!(g.name, "Тестовый сервер");
        assert_eq!(g.channels.as_ref().unwrap().len(), 6);
        assert_eq!(g.roles.len(), 2);
        assert_eq!(g.members.as_ref().unwrap().len(), 2);
        assert_eq!(g.presences.as_ref().unwrap().len(), 2);
        assert_eq!(g.voice_states.as_ref().unwrap().len(), 1);
        assert_eq!(g.emojis.as_ref().unwrap().len(), 2);
        assert_eq!(g.member_count, Some(42));
    }

    #[test]
    fn role_permissions_accepts_number() {
        let mut v = realistic_guild_create();
        // Discord местами шлёт permissions числом — роли не должны падать.
        v["roles"][0]["permissions"] = json!(137438953471u64);
        let g: Guild = serde_json::from_value(v).expect("строка и число в permissions");
        assert_eq!(g.roles[0].permissions, "137438953471");
    }

    #[test]
    fn guild_create_salvage_skips_broken_items() {
        let mut v = realistic_guild_create();
        // Два битых канала и битая роль — гильдия обязана спарситься,
        // минус ровно эти элементы.
        v["channels"][2]["id"] = json!(null); // id: null
        v["channels"][4]["type"] = json!("13"); // type строкой
        v["roles"][1]["name"] = json!(12345); // name числом
        assert!(serde_json::from_value::<Guild>(v.clone()).is_err());
        let g = Guild::salvage_from_value(&v).expect("salvage вытаскивает гильдию");
        assert_eq!(g.channels.as_ref().unwrap().len(), 4);
        assert_eq!(g.roles.len(), 1);
        assert_eq!(g.members.as_ref().unwrap().len(), 2);
        assert_eq!(g.presences.as_ref().unwrap().len(), 2);
        assert_eq!(g.voice_states.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn guild_salvage_requires_id() {
        let v = json!({"name": "Нет id"});
        assert!(Guild::salvage_from_value(&v).is_none());
    }

    /// Живой кейс v0.7.1: юзер-сессия прислала GUILD_CREATE БЕЗ
    /// верхнеуровневого name («missing field `name`» в строгом разборе),
    /// имя — в properties.name.
    #[test]
    fn guild_create_without_name_parses_and_falls_back_to_properties() {
        let mut v = realistic_guild_create();
        v["properties"] = json!({"name": "Имя из properties", "emoji_allowlist": []});
        v.as_object_mut().unwrap().remove("name");

        // Строгий разбор больше не падает: name опционален (= "").
        let g: Guild = serde_json::from_value(v.clone()).expect("name опционален (serde default)");
        assert_eq!(g.name, "");

        // Display-имя добирается из properties.name.
        assert_eq!(
            Guild::name_from_payload(&v).as_deref(),
            Some("Имя из properties")
        );

        // Salvage тоже тянет имя из properties.
        let s = Guild::salvage_from_value(&v).expect("salvage");
        assert_eq!(s.name, "Имя из properties");
        assert_eq!(s.channels.as_ref().map(Vec::len), Some(6));
    }

    /// Полностью безымянная ленивая гильдия: name_from_payload → None,
    /// salvage ставит «…» (обработчик возьмёт прежнее имя из кэша).
    #[test]
    fn guild_create_nameless_returns_none_for_display_name() {
        let mut v = realistic_guild_create();
        v.as_object_mut().unwrap().remove("name");
        assert!(Guild::name_from_payload(&v).is_none());
        let s = Guild::salvage_from_value(&v).expect("salvage");
        assert_eq!(s.name, "…");
    }

    /// v0.9.6 (скрин юзера): активная гильдия с каналами рендерилась в рейле
    /// инициалами — юзер-GUILD_CREATE держит иконку в properties.icon, как
    /// и имя в properties.name. Проверяем фолбэк и приоритет верхнеуровневой.
    #[test]
    fn guild_icon_falls_back_to_properties() {
        let mut v = realistic_guild_create();
        v["properties"] = json!({"name": "WoW Secrets", "icon": "a_abc123"});
        v.as_object_mut().unwrap().remove("icon");
        // Верхнеуровневого icon нет — берём из properties.
        assert_eq!(Guild::icon_from_payload(&v).as_deref(), Some("a_abc123"));
        // Верхнеуровневый icon приоритетнее.
        v["icon"] = json!("top_level_hash");
        assert_eq!(
            Guild::icon_from_payload(&v).as_deref(),
            Some("top_level_hash")
        );
        // Нигде нет — None; пустая строка — тоже None (не затираем кэш).
        v.as_object_mut().unwrap().remove("icon");
        v["properties"]["icon"] = json!("");
        assert!(Guild::icon_from_payload(&v).is_none());
        let mut bare = json!({"id": "123"});
        assert!(Guild::icon_from_payload(&bare).is_none());
        bare["icon"] = json!(null);
        assert!(Guild::icon_from_payload(&bare).is_none());
    }

    /// v0.9.0: каналы несут permission_overwrites — скрытые для роли каналы
    /// определяются по deny VIEW_CHANNEL (бит 10, 1024) на @everyone/роль.
    /// allow/deny приходят строкой, но местами числом — оба должны парситься.
    #[test]
    fn channel_overwrites_parse_string_and_number() {
        let mut v = realistic_guild_create();
        v["channels"][0]["permission_overwrites"] = json!([
            {"id": "918746238947562345", "type": 0,
             "allow": "0", "deny": "1024"},           // @everyone: deny VIEW_CHANNEL
            {"id": "918746238947562346", "type": 0,
             "allow": 1024, "deny": "0"},             // роль: allow (числом!)
            {"id": "111111111111111111", "type": 1,
             "allow": "1024", "deny": "0"}            // конкретный юзер
        ]);
        let g: Guild = serde_json::from_value(v).expect("overwrites парсятся");
        let chans = g.channels.unwrap();
        let ow = &chans[0].permission_overwrites;
        assert_eq!(ow.len(), 3);
        assert_eq!(ow[0].deny, "1024");
        assert_eq!(ow[1].allow, "1024", "allow числом тоже принимается");
        assert_eq!(ow[2].kind, 1);
    }
}
