//! Gateway DISPATCH routing: update the Rust cache, forward everything to
//! the UI, and assemble voice handshakes from the two-voice-event dance.

use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

use crate::rest::models::{
    Channel, Guild, Member, Presence, PresenceUser, Relationship, VoiceState,
};
use crate::state::{GuildState, MemberListRow, SharedState};
use crate::voice::VoiceControls;

/// Session state accumulated from DISPATCH events. Lives across reconnects
/// so the gateway loop can RESUME instead of re-IDENTIFYing.
#[derive(Default)]
pub struct SessionData {
    /// `session_id` from READY — until it arrives there is nothing to resume.
    pub id: Option<String>,
    /// `resume_gateway_url` from READY — dedicated websocket URL for RESUME.
    pub resume_url: Option<String>,
    /// Last `s` (sequence number) seen on a DISPATCH; sent with every
    /// heartbeat and replayed in RESUME so Discord can fill the gap.
    pub seq: Option<u64>,
}

pub async fn handle_dispatch(
    app: &tauri::AppHandle,
    state: &SharedState,
    t: &str,
    d: Value,
    session: &mut SessionData,
) {
    match t {
        "READY" => {
            let user_id = d["user"]["id"].as_str().unwrap_or_default().to_string();
            let username = d["user"]["username"].as_str().unwrap_or("?").to_string();
            let session_id = d["session_id"].as_str().unwrap_or_default().to_string();
            session.id = Some(session_id.clone());
            session.resume_url = d["resume_gateway_url"].as_str().map(String::from);
            if let Some(user) = d.get("user").cloned() {
                if let Ok(u) = serde_json::from_value(user.clone()) {
                    state.cache.write().unwrap().user = Some(u);
                }
            }
            let mut stub_ids: Vec<String> = Vec::new();
            {
                let mut cache = state.cache.write().unwrap();
                // Пользовательский READY (в отличие от бот-READY) несёт DM-каналы
                // прямо в private_channels — кладём их в кэш, чтобы UI сразу
                // видел список личных сообщений.
                if let Some(dms) = d.get("private_channels").and_then(|v| v.as_array()) {
                    for ch in dms {
                        if let Ok(c) = serde_json::from_value::<Channel>(ch.clone()) {
                            if !cache.dms.iter().any(|x| x.id == c.id) {
                                cache.dms.push(c);
                            }
                        }
                    }
                }
                // Друзья/заявки/блок-лист — тоже часть пользовательского READY.
                // Бот-аккаунты этот массив не получают.
                if let Some(rels) = d.get("relationships").and_then(|v| v.as_array()) {
                    for r in rels {
                        if let Ok(rel) = serde_json::from_value::<Relationship>(r.clone()) {
                            if let Some(uid) = r
                                .get("user")
                                .and_then(|u| u.get("id"))
                                .and_then(|v| v.as_str())
                            {
                                cache.relationships.insert(uid.to_string(), rel);
                            }
                        }
                    }
                }
                // READY.presences у юзер-сессии — это статусы друзей (без
                // guild_id); разкладываем их отдельно от гильдейских.
                if let Some(presences) = d.get("presences").and_then(|v| v.as_array()) {
                    for p in presences {
                        if let Ok(pr) = serde_json::from_value::<Presence>(p.clone()) {
                            if pr.guild_id.is_none() {
                                cache.friend_presences.insert(pr.user.id.clone(), pr);
                            }
                        }
                    }
                }
                // Гильдии из READY: некрупные приезжают ПОЛНЫМИ объектами
                // (каналы/роли/эмодзи прямо здесь — v0.9.7 это выбрасывал:
                // 64 из 79 серверов юзера оставались без каналов навсегда,
                // GUILD_CREATE приезжает только крупным после op 37),
                // крупные — ленивыми стабами (данные подтянутся их
                // GUILD_CREATE). Display-имя/иконка живут в верхнем уровне
                // ИЛИ в properties (CLIENT_STATE_V2) — как в GUILD_CREATE.
                let mut stubs_with_channels = 0usize;
                let mut stubs_with_props = 0usize;
                let mut stubs_named = 0usize;
                let mut stubs_with_icon = 0usize;
                if let Some(guilds) = d.get("guilds").and_then(|v| v.as_array()) {
                    for g in guilds {
                        let Some(id) = g.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        stub_ids.push(id.to_string());
                        if cache.guilds.contains_key(id) {
                            continue; // RESUME: полная гильдия уже в кэше
                        }
                        // Толерантный разбор записи: строгий serde → salvage
                        // (битый канал/роль выбрасывается, а не вся гильдия —
                        // как в GUILD_CREATE).
                        let parsed = serde_json::from_value::<Guild>(g.clone())
                            .ok()
                            .or_else(|| Guild::salvage_from_value(g));
                        let name = Guild::name_from_payload(g);
                        let icon = Guild::icon_from_payload(g);
                        if g.get("properties").is_some() {
                            stubs_with_props += 1;
                        }
                        if name.is_some() {
                            stubs_named += 1;
                        }
                        if icon.is_some() {
                            stubs_with_icon += 1;
                        }
                        let (roles, emojis, channels, threads, member_count) = match &parsed {
                            Some(pg) => {
                                if pg.channels.as_ref().is_some_and(|c| !c.is_empty()) {
                                    stubs_with_channels += 1;
                                }
                                (
                                    pg.roles.clone(),
                                    pg.emojis.clone().unwrap_or_default(),
                                    pg.channels.clone().unwrap_or_default(),
                                    pg.threads.clone().unwrap_or_default(),
                                    pg.member_count.unwrap_or(0),
                                )
                            }
                            None => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0),
                        };
                        cache.guilds.insert(
                            id.to_string(),
                            GuildState {
                                id: id.to_string(),
                                name: name.unwrap_or_else(|| "…".to_string()),
                                icon,
                                banner: parsed.as_ref().and_then(|pg| pg.banner.clone()),
                                owner_id: parsed
                                    .as_ref()
                                    .map(|pg| pg.owner_id.clone())
                                    .unwrap_or_default(),
                                roles,
                                emojis,
                                channels,
                                threads,
                                members: Default::default(),
                                presences: Default::default(),
                                voice_states: Vec::new(),
                                member_count,
                            },
                        );
                    }
                }
                if stub_ids.len() > 1 {
                    tracing::info!(
                        "gateway: READY-гильдии: {} шт., с каналами {}, с именем {}, \
                         с иконкой {}, с properties-обёрткой {}",
                        stub_ids.len(),
                        stubs_with_channels,
                        stubs_named,
                        stubs_with_icon,
                        stubs_with_props
                    );
                }
                // merged_members[i] / merged_presences параллельны d.guilds:
                // собственная карточка участника и статусы друзей по гильдиям.
                // ВАЖНО: вне зоны cache.write() — apply_merged берёт лок сам.
            }
            apply_merged(state, &d);
            let rel_count = d
                .get("relationships")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let dm_count = d
                .get("private_channels")
                .and_then(|v| v.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            // Состав payload, ушедшего во фронтенд (v0.9.9): по логу
            // видно, откуда UI берёт имена/статусы — users/presences
            // верхнего уровня против merged-массивов. Путь ["key", ""]
            // читает d["key"], иначе — d[key0][key1].
            let get_path = |path: [&str; 2]| -> Option<&Value> {
                let m = d.get(path[0])?;
                if path[1].is_empty() {
                    Some(m)
                } else {
                    m.get(path[1])
                }
            };
            let flat_len = |path: [&str; 2]| {
                get_path(path)
                    .and_then(|v| v.as_array())
                    .map_or(0, Vec::len)
            };
            let total_len = |path: [&str; 2]| {
                get_path(path).and_then(|v| v.as_array()).map_or(0, |a| {
                    a.iter()
                        .filter_map(|x| x.as_array())
                        .map(Vec::len)
                        .sum::<usize>()
                })
            };
            tracing::info!(
                "gateway: READY payload → UI: users={}, presences={}, \
                 merged_presences: друзей={} гильдейских={}, merged_members={} (карточек)",
                flat_len(["users", ""]),
                flat_len(["presences", ""]),
                flat_len(["merged_presences", "friends"]),
                total_len(["merged_presences", "guilds"]),
                total_len(["merged_members", ""]),
            );
            tracing::info!(
                "gateway: READY — {username} ({user_id}), session {session_id}, {guild_n} гильдий, {rel_count} отношений, {dm_count} ЛС",
                guild_n = stub_ids.len(),
            );
            let _ = app.emit(
                "discord://ready",
                json!({ "user": d["user"], "session_id": session_id }),
            );
            // Раскладка рейла (папки серверов): современный Discord хранит
            // настройки в protobuf (user_settings_proto, base64), legacy —
            // JSON guild_folders (топ-уровень или внутри user_settings).
            // Схема прото свёрена с бандлом официального клиента и живёт
            // в settings_proto.rs. Единое событие для UI в обоих случаях.
            if !state.is_bot.load(Ordering::SeqCst) {
                emit_guild_layout(app, &d);
            }
            // ГЛАВНЫЙ ФИКС видимости серверов: юзер-сессии получают
            // GUILD_CREATE только на подписанные гильдии (op 37). Шторм
            // одним пакетом подозревался в обрывах — поэтому батчами по 50
            // с паузой 350 мс (мягче, чем один мегапакет и чем залп op 14
            // по всем гильдиям сразу, который валил сессию в v0.5.0).
            if !state.is_bot.load(Ordering::SeqCst) && !stub_ids.is_empty() {
                let tx = state.gateway_tx.read().unwrap().clone();
                if let Some(tx) = tx {
                    let st = Arc::clone(state);
                    let batches: Vec<Vec<String>> =
                        stub_ids.chunks(50).map(|c| c.to_vec()).collect();
                    tauri::async_runtime::spawn(async move {
                        for batch in batches {
                            if st.shutdown.read().unwrap().is_cancelled() {
                                break;
                            }
                            if tx
                                .send(crate::state::GatewayCommand::BulkGuildSubscribe {
                                    guild_ids: batch.clone(),
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                            tracing::info!(
                                "gateway: op 37 bulk subscribe — {} гильдий",
                                batch.len()
                            );
                            tokio::time::sleep(Duration::from_millis(350)).await;
                        }
                    });
                }
            }
            // После переподключения сервер «забывает» подписки на списки
            // участников (op 37 с channels) — bulk выше покрывает только
            // события гильдий. Переигрываем подписки каналов, как
            // CONNECTION_OPEN → flush в официальном клиенте.
            replay_member_list_subscriptions(state);
            // Полный READY вперёд как dispatch: фронтенду нужны
            // private_channels (recipient_ids!), users, relationships
            // и presences, которых нет в коротком ready-событии выше.
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
            state.emit_status("connected", None);
        }

        "RESUMED" => {
            // RESUME прошёл: сессия продолжается, но сервер мог потерять
            // подписки ленивых списков — официальный клиент делает flush
            // подписок на CONNECTION_RESUMED ровно как на CONNECTION_OPEN.
            replay_member_list_subscriptions(state);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
            state.emit_status("connected", None);
        }

        "READY_SUPPLEMENTAL" => {
            // Приезжает сразу после потока GUILD_CREATE: голосовые состояния
            // гильдий, merged_members (своя карточка) и merged_presences
            // (статусы друзей и «видимые» участники по гильдиям).
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(guilds) = d.get("guilds").and_then(|v| v.as_array()) {
                    for g in guilds {
                        let Some(id) = g.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        let Some(gs) = cache.guilds.get_mut(id) else {
                            continue;
                        };
                        if let Some(vs) = g.get("voice_states") {
                            if let Ok(list) = serde_json::from_value::<Vec<VoiceState>>(vs.clone())
                            {
                                gs.voice_states = list;
                            }
                        }
                    }
                }
            }
            apply_merged(state, &d);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "RELATIONSHIP_ADD" => {
            // {id, type, user, nickname?} — id совпадает с user.id.
            if let Ok(rel) = serde_json::from_value::<Relationship>(d.clone()) {
                let uid = rel.user.id.clone();
                let kind = rel.kind;
                state
                    .cache
                    .write()
                    .unwrap()
                    .relationships
                    .insert(uid.clone(), rel);
                tracing::debug!("gateway: RELATIONSHIP_ADD {uid} (type {kind})");
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "RELATIONSHIP_REMOVE" => {
            let uid = d["id"].as_str().unwrap_or_default().to_string();
            if !uid.is_empty() {
                let mut cache = state.cache.write().unwrap();
                cache.relationships.remove(&uid);
                cache.friend_presences.remove(&uid);
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_CREATE" => {
            // d делаем mut сразу: после обогащения имени/иконки payload
            // нормализуется перед отправкой в UI (см. ниже).
            let mut d = d;
            // Мягкий разбор: одна кривая гильдия не должна ронять ВСЮ сессию
            // (раньше `?` здесь убивал соединение → бесконечный reconnect).
            // Двухступенчато: строгий serde-разбор, а при неудаче — salvage
            // с поэлементным разбором коллекций (битый канал/роль/участник
            // выбрасывается, а не вся гильдия). Точная причина строго
            // неудачного разбора пишется в лог — это главная зацепка
            // для диагностики нестандартных payload'ов юзер-сессий.
            let parsed: Option<Guild> = match serde_json::from_value::<Guild>(d.clone()) {
                Ok(g) => Some(g),
                Err(e) => {
                    tracing::warn!("gateway: GUILD_CREATE строгий разбор не прошёл: {e}");
                    Guild::salvage_from_value(&d)
                }
            };
            let needs_chunk = parsed
                .as_ref()
                .map_or(true, |g| g.members.as_ref().map_or(true, |m| m.is_empty()));
            if let Some(g) = parsed {
                // Юзер-сессии шлют GUILD_CREATE без верхнеуровневого name —
                // display-имя живёт в properties.name, а если и его нет
                // (ленивая гильдия) — остаётся прежнее имя из кэша (стаб
                // READY). Иконка — та же логика: пустую не пишем поверх
                // существующей, иначе рейл теряет картинки серверов.
                let mut name = if g.name.is_empty() {
                    Guild::name_from_payload(&d).unwrap_or_default()
                } else {
                    g.name.clone()
                };
                // Иконка — та же логика, что и имя: юзер-GUILD_CREATE
                // может держать её в properties.icon; пустую не пишем
                // поверх существующей, иначе рейл теряет картинки.
                let mut icon = g.icon.clone().or_else(|| Guild::icon_from_payload(&d));
                {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(old) = cache.guilds.get(&g.id) {
                        if name.is_empty() && !old.name.is_empty() && old.name != "…" {
                            name = old.name.clone();
                        }
                        if icon.is_none() {
                            icon = old.icon.clone();
                        }
                    }
                    if name.is_empty() {
                        name = "Без названия".to_string();
                    }
                    let mut gs = GuildState {
                        id: g.id.clone(),
                        name: name.clone(),
                        icon: icon.clone(),
                        banner: g.banner.clone(),
                        owner_id: g.owner_id.clone(),
                        roles: g.roles.clone(),
                        emojis: g.emojis.clone().unwrap_or_default(),
                        channels: Vec::new(),
                        threads: g.threads.clone().unwrap_or_default(),
                        members: Default::default(),
                        presences: Default::default(),
                        voice_states: Vec::new(),
                        member_count: g.member_count.unwrap_or(0),
                    };
                    if let Some(chs) = &g.channels {
                        gs.channels = chs.clone();
                    }
                    if let Some(members) = &g.members {
                        for m in members {
                            if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                                gs.members.insert(uid, m.clone());
                            }
                        }
                    }
                    if let Some(presences) = &g.presences {
                        for p in presences {
                            gs.presences.insert(p.user.id.clone(), p.clone());
                        }
                    }
                    if let Some(vs) = &g.voice_states {
                        gs.voice_states = vs.clone();
                    }
                    cache.guilds.insert(g.id.clone(), gs);
                }
                tracing::info!(
                    "gateway: GUILD_CREATE «{}» {} ({} каналов)",
                    name,
                    g.id,
                    g.channels.as_ref().map(Vec::len).unwrap_or(0)
                );
                // op 14-подписка на список участников — ТОЛЬКО по запросу
                // (юзер-сессии: commands::subscribe_guild при открытии сервера).
                // Ботам оставлен привычный op 8 на полную выгрузку участников.
                if needs_chunk && state.is_bot.load(Ordering::SeqCst) {
                    if let Some(tx) = state.gateway_tx.read().unwrap().as_ref() {
                        let _ = tx.try_send(crate::state::GatewayCommand::RequestMembers {
                            guild_id: g.id.clone(),
                        });
                    }
                }
                // Нормализуем payload для UI: вписываем добытое имя/иконку,
                // чтобы фронтенд не увидел гильдию без name (его стаб
                // с настоящим именем иначе перетирается безымянной).
                if let Some(obj) = d.as_object_mut() {
                    let has_name = obj
                        .get("name")
                        .and_then(|x| x.as_str())
                        .is_some_and(|s| !s.is_empty());
                    if !has_name {
                        obj.insert("name".to_string(), json!(name));
                    }
                    let has_icon = obj
                        .get("icon")
                        .and_then(|x| x.as_str())
                        .is_some_and(|s| !s.is_empty());
                    if icon.is_some() && !has_icon {
                        obj.insert("icon".to_string(), json!(icon));
                    }
                }
            } else {
                tracing::warn!(
                    "gateway: GUILD_CREATE не разобран даже salvage (нет id) — прокидываем сырое событие в UI"
                );
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBERS_CHUNK" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let members: Vec<Member> =
                serde_json::from_value(d["members"].clone()).unwrap_or_default();
            let chunk_index = d["chunk_index"].as_u64().unwrap_or(0);
            let chunk_count = d["chunk_count"].as_u64().unwrap_or(1);
            // Ответ на op 14 (activities: true) несёт и пресенсы — без них
            // список участников выглядит «все офлайн».
            let presences: Vec<Presence> =
                serde_json::from_value(d["presences"].clone()).unwrap_or_default();
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    for m in members {
                        if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                            gs.members.insert(uid, m);
                        }
                    }
                    for p in presences {
                        gs.presences.insert(p.user.id.clone(), p);
                    }
                }
            }
            tracing::debug!(
                "gateway: чанк участников {}/{} для гильдии {}",
                chunk_index + 1,
                chunk_count,
                guild_id
            );
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBER_LIST_UPDATE" => {
            // Ленивый список участников — ответ на op 37 с channels
            // (subscribe_channel / псевдоканал "0"). Формат сверен с
            // бандлом discord.com (flux-стор GuildMemberListStore):
            //   d.id           — СЕРВЕРНЫЙ id списка: "everyone" для
            //                   обычных каналов (без @everyone-deny на
            //                   VIEW_CHANNEL), вычисленный — для приватных;
            //                   это НЕ id канала (урок v0.9.5)
            //   d.member_count — всего участников
            //   d.groups       — [{id: "online"|"offline"|role_id, count}]
            //   d.ops — инкрементальные операции:
            //     {op:"SYNC", range:[a,b], items:[{group}|{member}, …]}
            //     {op:"INSERT", index, item}  {op:"UPDATE", index, item}
            //     {op:"DELETE", index}        {op:"INVALIDATE", range}
            //   member = {user, roles, presence{status,activities},
            //             hoisted_role, nick}. Сервер присылает ГОТОВЫЙ
            // сайдбар: группы и участники уже в правильном порядке —
            // применяем ops и шлём снапшот в UI (раз в событие, дёшево).
            let guild_id_opt = d.get("guild_id").and_then(|v| v.as_str()).map(String::from);
            if let Some(guild_id) = guild_id_opt {
                let list_id = d
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                let member_count = d.get("member_count").and_then(|v| v.as_u64()).unwrap_or(0);
                // SYNC-op в событии — авторитетный снимок сервера (для
                // escape-hatch трекера: ошибочное предсеяние 'everyone' у
                // роль-deny каналов самолечится перелокизацией).
                let has_sync = d.get("ops").and_then(|v| v.as_array()).is_some_and(|ops| {
                    ops.iter()
                        .any(|op| op.get("op").and_then(|x| x.as_str()) == Some("SYNC"))
                });

                // {group:{id,count}} | {member:{…}} → строка снапшота.
                let parse_item = |item: &Value| -> Option<MemberListRow> {
                    if let Some(g) = item.get("group") {
                        return Some(MemberListRow::Group {
                            id: g
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            count: g.get("count").and_then(|v| v.as_u64()).unwrap_or(0),
                        });
                    }
                    let m = item.get("member")?;
                    let user = m.get("user")?;
                    let uid = user
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if uid.is_empty() {
                        return None;
                    }
                    let presence = m.get("presence");
                    Some(MemberListRow::Member {
                        user_id: uid,
                        username: user
                            .get("username")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                            .to_string(),
                        global_name: user
                            .get("global_name")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        nick: m.get("nick").and_then(|v| v.as_str()).map(String::from),
                        discriminator: user
                            .get("discriminator")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        avatar: user
                            .get("avatar")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        bot: user.get("bot").and_then(|v| v.as_bool()).unwrap_or(false),
                        status: presence
                            .and_then(|p| p.get("status"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("offline")
                            .to_string(),
                        activities: presence
                            .and_then(|p| p.get("activities"))
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default(),
                        roles: m
                            .get("roles")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        hoisted_role: m
                            .get("hoisted_role")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                };

                // Собираем (uid, member) для гидратации кэша — как eY/eZ
                // в официальном клиенте (UserStore+PresenceStore): статусы
                // отсюда видят профиль, ЛС и «кто в сети». Гидратация — из
                // событий ЛЮБОГО канала: сервер держит подписки прежних
                // каналов до LRU-вытеснения и шлёт их обновления тоже.
                let mut hydrate: Vec<(String, Value)> = Vec::new();
                let scan = |item: &Value, out: &mut Vec<(String, Value)>| {
                    if let Some(m) = item.get("member") {
                        if let Some(uid) = m
                            .get("user")
                            .and_then(|u| u.get("id"))
                            .and_then(|v| v.as_str())
                        {
                            out.push((uid.to_string(), m.clone()));
                        }
                    }
                };
                if let Some(ops) = d.get("ops").and_then(|v| v.as_array()) {
                    for op in ops {
                        match op.get("op").and_then(|v| v.as_str()).unwrap_or("") {
                            "SYNC" => {
                                if let Some(items) = op.get("items").and_then(|v| v.as_array()) {
                                    for it in items {
                                        scan(it, &mut hydrate);
                                    }
                                }
                            }
                            "INSERT" | "UPDATE" => {
                                if let Some(item) = op.get("item") {
                                    scan(item, &mut hydrate);
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Снапшот строк — только для ТЕКУЩЕГО канала (его зафикси-
                // ровал subscribe_channel заранее): события прежних
                // каналов не должны затирать сайдбар — иначе список
                // мерцает между каналами при быстром переключении.
                let mut rows_changed = false;
                let is_current;
                let rows_after: usize;
                {
                    let mut lists = state.member_lists.lock().unwrap();
                    let list = lists.entry(guild_id.clone()).or_default();
                    if list.channel_id.is_empty() {
                        // Гонка «событие раньше команды» (или первый список
                        // гильдии после старта) — принимаем как текущий.
                        list.channel_id = list_id.clone();
                    }
                    // УРОК v0.9.5: серверный id списка (d.id) — НЕ id канала.
                    // Обычные каналы (без @everyone-deny VIEW_CHANNEL) делят
                    // общий список "everyone", приватные получают вычисленный
                    // id. Трекер (предсеянный в subscribe_channel по overwrites
                    // или выученной карте канал→список) матчит события и глушит
                    // хвосты старых подписок (сервер держит до 5 каналов).
                    is_current = list.tracker.accept(&list_id, has_sync);
                    if is_current {
                        // Сервер подтвердил id списка этой гильдии — предсеем
                        // им следующие каналы (скрытые списки: один вычисленный
                        // id на все каналы, SYNC для них повторно не приедет).
                        list.guild_list = Some(list_id.clone());
                        // Выучить id списка канала: повторные визиты — без
                        // ожидания SYNC (карта переживает переключения).
                        if !list.channel_id.is_empty() {
                            list.channel_lists
                                .insert(list.channel_id.clone(), list_id.clone());
                        }
                        // Строки чужого списка (переключение канала) —
                        // спрятать/восстановить из мини-кэша per-list.
                        if list.rows_list.as_deref() != Some(list_id.as_str()) {
                            let prev = list.rows_list.clone();
                            list.switch_to_list(Some(list_id.as_str()));
                            if prev.is_some() {
                                tracing::info!(
                                    "member-list: {guild_id} сменили список \
                                     {prev:?} → {list_id} (SYNC={has_sync})"
                                );
                            }
                        }
                    }
                    if is_current {
                        if list.member_count != member_count {
                            list.member_count = member_count;
                            rows_changed = true;
                        }
                        if let Some(groups) = d.get("groups").and_then(|v| v.as_array()) {
                            let online = groups
                                .iter()
                                .find(|g| g.get("id").and_then(|v| v.as_str()) == Some("online"))
                                .and_then(|g| g.get("count").and_then(|v| v.as_u64()))
                                .unwrap_or(0);
                            if list.online_count != online {
                                list.online_count = online;
                                rows_changed = true;
                            }
                        }
                        if let Some(ops) = d.get("ops").and_then(|v| v.as_array()) {
                            for op in ops {
                                let kind = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
                                match kind {
                                    "SYNC" => {
                                        // Официальный sync(range, items):
                                        // rows[start + i] = item — ПРИСВАИВАНИЕ
                                        // по индексу с удлинением списка
                                        // (сверено с бандлом discord.com:
                                        // t.forEach((e,t)=>this.update(n+t,e))).
                                        // Старые строки за пределами новых
                                        // items НЕ трогаем — удаления едят
                                        // отдельными DELETE/INVALIDATE ops.
                                        // Диапазоны ходят подряд ([0,99],
                                        // потом [100,199]…), поэтому дырок
                                        // не бывает; start > len — дописываем
                                        // в конец (защита вместо JS-дырок).
                                        let items: Vec<MemberListRow> = op
                                            .get("items")
                                            .and_then(|v| v.as_array())
                                            .map(|arr| arr.iter().filter_map(parse_item).collect())
                                            .unwrap_or_default();
                                        let start = op
                                            .get("range")
                                            .and_then(|r| r.as_array())
                                            .and_then(|r| r.first())
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0)
                                            as usize;
                                        if start >= list.rows.len() {
                                            list.rows.extend(items);
                                        } else {
                                            for (i, item) in items.into_iter().enumerate() {
                                                let idx = start + i;
                                                if idx < list.rows.len() {
                                                    list.rows[idx] = item;
                                                } else {
                                                    list.rows.push(item);
                                                }
                                            }
                                        }
                                        rows_changed = true;
                                    }
                                    "INSERT" => {
                                        let idx = op.get("index").and_then(|v| v.as_u64());
                                        if let (Some(idx), Some(item)) = (idx, op.get("item")) {
                                            if let Some(row) = parse_item(item) {
                                                let idx = (idx as usize).min(list.rows.len());
                                                list.rows.insert(idx, row);
                                                rows_changed = true;
                                            }
                                        }
                                    }
                                    "UPDATE" => {
                                        let idx = op.get("index").and_then(|v| v.as_u64());
                                        if let (Some(idx), Some(item)) = (idx, op.get("item")) {
                                            let idx = idx as usize;
                                            if idx < list.rows.len() {
                                                if let Some(row) = parse_item(item) {
                                                    list.rows[idx] = row;
                                                    rows_changed = true;
                                                }
                                            }
                                        }
                                    }
                                    "DELETE" => {
                                        if let Some(idx) = op.get("index").and_then(|v| v.as_u64())
                                        {
                                            let idx = idx as usize;
                                            if idx < list.rows.len() {
                                                list.rows.remove(idx);
                                                rows_changed = true;
                                            }
                                        }
                                    }
                                    "INVALIDATE" => {
                                        // Строки диапазона вышли из зоны подписки.
                                        if let Some(range) =
                                            op.get("range").and_then(|v| v.as_array())
                                        {
                                            let start =
                                                range.first().and_then(|v| v.as_u64()).unwrap_or(0)
                                                    as usize;
                                            let end =
                                                range.get(1).and_then(|v| v.as_u64()).unwrap_or(0)
                                                    as usize;
                                            if start <= end && start < list.rows.len() {
                                                let end = (end + 1).min(list.rows.len());
                                                list.rows.drain(start..end);
                                                rows_changed = true;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    rows_after = list.rows.len();
                }
                // Диагностика (v0.9.8): SYNC — авторитетный снимок сервера;
                // каждый логируем в файл — «список не грузится» теперь
                // разбирается по логу (пришёл ли SYNC, принят ли, сколько
                // строк получилось), без угадываний. v0.9.9: + сколько
                // карточек участников гидратируется в кэш гильдии (имена/
                // статусы для профиля и «кто в сети»).
                if has_sync {
                    tracing::info!(
                        "member-list: {guild_id} SYNC список={list_id} принято={is_current} \
                         строк={rows_after} карточек={}",
                        hydrate.len()
                    );
                } else if !is_current {
                    tracing::debug!("member-list: {guild_id} хвост списка {list_id} пропущен");
                }
                if !hydrate.is_empty() {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                        if member_count > 0 {
                            gs.member_count = member_count;
                        }
                        for (uid, m) in hydrate {
                            // Presence: статус участника — в кэш гильдии
                            // (профили/ЛС/«кто в сети» берут его отсюда).
                            let presence_val = json!({
                                "user": {
                                    "id": uid,
                                    "username": m["user"]["username"].clone(),
                                    "discriminator": m["user"]["discriminator"].clone(),
                                    "global_name": m["user"]["global_name"].clone(),
                                    "avatar": m["user"]["avatar"].clone(),
                                },
                                "guild_id": guild_id,
                                "status": m["presence"]["status"].clone(),
                                "activities": m["presence"]["activities"].clone(),
                            });
                            if let Ok(p) = serde_json::from_value::<Presence>(presence_val) {
                                gs.presences.insert(uid.clone(), p);
                            }
                            // Карточку участника не затираем, только добавляем
                            // отсутствующую (в кэше бывают более полные).
                            if let Ok(member) = serde_json::from_value::<Member>(m.clone()) {
                                gs.members.entry(uid.clone()).or_insert(member);
                            }
                        }
                    }
                }
                // Снапшот в UI — только для текущего канала (чужие списки
                // не должны перебивать сайдбар активного канала).
                if is_current {
                    let lists = state.member_lists.lock().unwrap();
                    if let Some(list) = lists.get(&guild_id) {
                        let _ = app.emit(
                            "discord://member-list",
                            json!({
                                "guild_id": guild_id,
                                "channel_id": list.channel_id,
                                "member_count": list.member_count,
                                "online_count": list.online_count,
                                "rows": list.rows,
                            }),
                        );
                        if rows_changed {
                            tracing::debug!(
                                "gateway: member list {} — {} строк (канал {}, онлайн {}, всего {})",
                                guild_id,
                                list.rows.len(),
                                list.channel_id,
                                list.online_count,
                                list.member_count
                            );
                        }
                    }
                }
            }
        }

        "GUILD_MEMBER_ADD" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Ok(m) = serde_json::from_value::<Member>(d["member"].clone()) {
                if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                        gs.members.insert(uid, m);
                        gs.member_count += 1;
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBER_UPDATE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Ok(m) = serde_json::from_value::<Member>(d.clone()) {
                if let Some(uid) = m.user.as_ref().map(|u| u.id.clone()) {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                        gs.members.insert(uid, m);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_MEMBER_REMOVE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let uid = d["user"]["id"].as_str().unwrap_or_default().to_string();
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    gs.members.remove(&uid);
                    gs.member_count = gs.member_count.saturating_sub(1);
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_UPDATE" => {
            if let Ok(g) = serde_json::from_value::<Guild>(d.clone()) {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&g.id) {
                    gs.name = g.name;
                    gs.icon = g.icon;
                    gs.banner = g.banner;
                    gs.owner_id = g.owner_id;
                    gs.roles = g.roles;
                    gs.emojis = g.emojis.unwrap_or_default();
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_DELETE" => {
            let gid = d["id"].as_str().unwrap_or_default().to_string();
            state.cache.write().unwrap().guilds.remove(&gid);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Ok(role) = serde_json::from_value::<crate::rest::models::Role>(d["role"].clone())
            {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                    gs.roles.retain(|r| r.id != role.id);
                    gs.roles.push(role);
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "GUILD_ROLE_DELETE" => {
            let guild_id = d["guild_id"].as_str().unwrap_or_default().to_string();
            let role_id = d["role_id"].as_str().unwrap_or_default().to_string();
            let mut cache = state.cache.write().unwrap();
            if let Some(gs) = cache.guilds.get_mut(&guild_id) {
                gs.roles.retain(|r| r.id != role_id);
            }
            drop(cache);
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "CHANNEL_CREATE" => {
            if let Ok(c) = serde_json::from_value::<Channel>(d.clone()) {
                let is_dm = c.guild_id.is_none() && (c.kind == 1 || c.kind == 3);
                if is_dm {
                    let mut cache = state.cache.write().unwrap();
                    if !cache.dms.iter().any(|x| x.id == c.id) {
                        cache.dms.push(c);
                    }
                } else if let Some(gid) = &c.guild_id {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(gid) {
                        if !gs.channels.iter().any(|x| x.id == c.id) {
                            gs.channels.push(c);
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "CHANNEL_UPDATE" => {
            if let Ok(c) = serde_json::from_value::<Channel>(d.clone()) {
                if let Some(gid) = &c.guild_id {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(gid) {
                        if let Some(slot) = gs.channels.iter_mut().find(|x| x.id == c.id) {
                            *slot = c;
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "CHANNEL_DELETE" => {
            if let Ok(c) = serde_json::from_value::<Channel>(d.clone()) {
                let mut cache = state.cache.write().unwrap();
                cache.dms.retain(|x| x.id != c.id);
                if let Some(gid) = &c.guild_id {
                    if let Some(gs) = cache.guilds.get_mut(gid) {
                        gs.channels.retain(|x| x.id != c.id);
                        // Треды удалённого родителя больше не показываем.
                        if matches!(c.kind, 0 | 5 | 15) {
                            let pid = c.id.clone();
                            gs.threads
                                .retain(|t| t.parent_id.as_deref() != Some(pid.as_str()));
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        // Треды (v0.8.0): живут отдельной коллекцией гильдии.
        "THREAD_CREATE" => {
            if let Ok(th) = serde_json::from_value::<Channel>(d.clone()) {
                if let Some(gid) = th.guild_id.clone() {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&gid) {
                        gs.threads.retain(|x| x.id != th.id);
                        gs.threads.push(th);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "THREAD_UPDATE" => {
            if let Ok(th) = serde_json::from_value::<Channel>(d.clone()) {
                if let Some(gid) = th.guild_id.clone() {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&gid) {
                        if let Some(slot) = gs.threads.iter_mut().find(|x| x.id == th.id) {
                            *slot = th;
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "THREAD_DELETE" => {
            let gid = d["guild_id"].as_str().unwrap_or_default().to_string();
            let tid = d["id"].as_str().unwrap_or_default().to_string();
            if !gid.is_empty() && !tid.is_empty() {
                let mut cache = state.cache.write().unwrap();
                if let Some(gs) = cache.guilds.get_mut(&gid) {
                    gs.threads.retain(|x| x.id != tid);
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "THREAD_LIST_SYNC" => {
            // Полный снимок активных тредов гильдии: мержим в кэш.
            let gid = d["guild_id"].as_str().unwrap_or_default().to_string();
            if let Some(list) = d.get("threads").and_then(|v| v.as_array()) {
                let mut fresh: Vec<Channel> = Vec::new();
                for th in list {
                    if let Ok(c) = serde_json::from_value::<Channel>(th.clone()) {
                        fresh.push(c);
                    }
                }
                if !gid.is_empty() {
                    let mut cache = state.cache.write().unwrap();
                    if let Some(gs) = cache.guilds.get_mut(&gid) {
                        for c in fresh {
                            gs.threads.retain(|x| x.id != c.id);
                            gs.threads.push(c);
                        }
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "PRESENCE_UPDATE" => {
            if let Ok(p) = serde_json::from_value::<Presence>(d.clone()) {
                match p.guild_id.clone() {
                    Some(gid) => {
                        let uid = p.user.id.clone();
                        let mut cache = state.cache.write().unwrap();
                        if let Some(gs) = cache.guilds.get_mut(&gid) {
                            gs.presences.insert(uid, p);
                        }
                    }
                    None => {
                        // Дружеский статус (вне гильдий) — обновляем кэш друзей.
                        let uid = p.user.id.clone();
                        state.cache.write().unwrap().friend_presences.insert(uid, p);
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "VOICE_STATE_UPDATE" => {
            if let Ok(vs) = serde_json::from_value::<VoiceState>(d.clone()) {
                match vs.guild_id.clone() {
                    None => {
                        // Звонок в ЛС/группе: guild_id отсутствует. Своё эхо op4
                        // сшивает рукопожатие по channel_id (voice-сервер
                        // идентифицирует DM-сессию по каналу); чужие состояния
                        // просто едут в UI (участники звонка).
                        let is_self = state.self_id().as_deref() == Some(vs.user_id.as_str());
                        if is_self {
                            match vs.channel_id.clone() {
                                Some(cid) => {
                                    // Вход/флаги в звонке ЛС: ключ = channel_id.
                                    if let Some(sid) = vs.session_id.clone() {
                                        let mut hs = state.voice_handshake.lock().unwrap();
                                        let entry = hs.entry(cid.clone()).or_default();
                                        entry.guild_id = cid.clone(); // songbird server_id = канал
                                        entry.user_id = vs.user_id.clone();
                                        entry.session_id = Some(sid);
                                        entry.channel_id = Some(cid.clone());
                                        entry.is_dm = true;
                                    }
                                    let sessions = state.voice_sessions.lock().unwrap();
                                    if let Some(handle) = sessions.get(&cid) {
                                        *handle.controls.lock().unwrap() =
                                            VoiceControls::new(vs.self_mute, vs.self_deaf);
                                        if let Some(mut driver) = handle.driver.clone() {
                                            driver.mute(vs.self_mute || vs.self_deaf);
                                        }
                                        // Мьют-флаги — главный подозреваемый «меня не
                                        // слышно»: mute=true шлёт тишину вместо микрофона.
                                        tracing::info!(
                                            "voice: op4-эхо (звонок {cid}): self_mute={} \
                                             self_deaf={} → driver.mute({})",
                                            vs.self_mute,
                                            vs.self_deaf,
                                            vs.self_mute || vs.self_deaf
                                        );
                                    }
                                }
                                None => {
                                    // Выход из звонка ЛС/группы: гасим DM-сессию.
                                    let dm_keys: Vec<String> = state
                                        .voice_handshake
                                        .lock()
                                        .unwrap()
                                        .iter()
                                        .filter(|(_, h)| h.is_dm)
                                        .map(|(k, _)| k.clone())
                                        .collect();
                                    for k in &dm_keys {
                                        state.voice_handshake.lock().unwrap().remove(k);
                                        state.voice_sessions.lock().unwrap().remove(k);
                                        state.voice_desired.lock().unwrap().remove(k);
                                        let mut tasks = state.voice_tasks.lock().await;
                                        if let Some((handle, token)) = tasks.remove(k) {
                                            token.cancel();
                                            handle.abort();
                                        }
                                        let _ = app.emit(
                                            "discord://voice",
                                            json!({ "kind": "disconnected", "data": { "guild_id": k, "dm": true } }),
                                        );
                                    }
                                }
                            }
                        }
                        state.voice_notify.notify_one();
                    }
                    Some(gid) => {
                        {
                            let mut cache = state.cache.write().unwrap();
                            if let Some(gs) = cache.guilds.get_mut(&gid) {
                                gs.voice_states.retain(|v| v.user_id != vs.user_id);
                                if vs.channel_id.is_some() {
                                    gs.voice_states.push(vs.clone());
                                }
                            }
                        }
                        // Голосовое рукопожатие для нас самих: запоминаем session_id.
                        let is_self = state.self_id().as_deref() == Some(vs.user_id.as_str());
                        if is_self {
                            if let (Some(sid), true) =
                                (vs.session_id.clone(), vs.channel_id.is_some())
                            {
                                let mut hs = state.voice_handshake.lock().unwrap();
                                let entry = hs.entry(gid.clone()).or_default();
                                entry.guild_id = gid.clone();
                                entry.user_id = vs.user_id.clone();
                                entry.session_id = Some(sid);
                                entry.channel_id = vs.channel_id.clone();
                                drop(hs);
                                // Эхо op4: подтягиваем живую сессию к актуальным флагам.
                                let sessions = state.voice_sessions.lock().unwrap();
                                if let Some(handle) = sessions.get(&gid) {
                                    *handle.controls.lock().unwrap() =
                                        VoiceControls::new(vs.self_mute, vs.self_deaf);
                                    // Мьют в songbird: движок шлёт тишину вместо микрофона
                                    // (деафиен тоже глушит передачу — как в оригинале).
                                    // Driver — клон канала управления, mute уходит в задачи.
                                    if let Some(mut driver) = handle.driver.clone() {
                                        driver.mute(vs.self_mute || vs.self_deaf);
                                    }
                                    // Мьют-флаги — главный подозреваемый «меня не слышно»:
                                    // mute=true шлёт тишину вместо микрофона.
                                    tracing::info!(
                                        "voice: op4-эхо (гильдия {gid}): self_mute={} \
                                         self_deaf={} → driver.mute({})",
                                        vs.self_mute,
                                        vs.self_deaf,
                                        vs.self_mute || vs.self_deaf
                                    );
                                }
                            } else if vs.channel_id.is_none() {
                                // Выход из войса: гасим голосовую сессию.
                                state.voice_handshake.lock().unwrap().remove(&gid);
                                state.voice_sessions.lock().unwrap().remove(&gid);
                                state.voice_desired.lock().unwrap().remove(&gid);
                                let mut tasks = state.voice_tasks.lock().await;
                                if let Some((handle, token)) = tasks.remove(&gid) {
                                    token.cancel();
                                    handle.abort();
                                }
                                let _ = app.emit(
                                    "discord://voice",
                                    json!({ "kind": "disconnected", "data": { "guild_id": gid } }),
                                );
                            }
                        }
                        state.voice_notify.notify_one();
                    }
                }
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        "VOICE_SERVER_UPDATE" => {
            let gid = d["guild_id"].as_str().unwrap_or_default().to_string();
            let token = d["token"].as_str().unwrap_or_default().to_string();
            let endpoint = d["endpoint"].as_str().unwrap_or_default().to_string();
            if !gid.is_empty() && !token.is_empty() {
                let user_id = state.self_id().unwrap_or_default();
                let mut hs = state.voice_handshake.lock().unwrap();
                let entry = hs.entry(gid.clone()).or_default();
                entry.guild_id = gid.clone();
                entry.user_id = user_id;
                entry.token = Some(token);
                entry.endpoint = Some(endpoint);
            } else if !token.is_empty() {
                // Звонок в ЛС/группе: guild_id = null — заполняем единственное
                // незавершённое DM-рукопожатие (голосовая сессия одна за раз).
                let user_id = state.self_id().unwrap_or_default();
                let mut hs = state.voice_handshake.lock().unwrap();
                if let Some((_, entry)) = hs.iter_mut().find(|(_, h)| h.is_dm && h.token.is_none())
                {
                    entry.user_id = user_id;
                    entry.token = Some(token);
                    entry.endpoint = Some(endpoint);
                    tracing::info!(
                        "gateway: VOICE_SERVER_UPDATE для звонка ЛС (канал {})",
                        entry.guild_id
                    );
                }
            }
            state.voice_notify.notify_one();
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }

        // Сообщения, реакции, typing и всё остальное идут в UI напрямую —
        // кэш сообщений живёт на стороне фронтенда (как IndexedDB у оригинала).
        "USER_SETTINGS_PROTO_UPDATE" => {
            // Современный формат настроек: base64-protobuf. Папки серверов
            // могли перестроиться (в другом клиенте) — пересылаем раскладку.
            if !state.is_bot.load(Ordering::SeqCst) {
                emit_guild_layout(app, &d);
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }
        "USER_SETTINGS_UPDATE" => {
            // Legacy-формат: раскладка может прийти прямо JSON'ом.
            if !state.is_bot.load(Ordering::SeqCst) {
                emit_guild_layout(app, &d);
            }
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }
        _ => {
            let _ = app.emit("discord://dispatch", json!({ "t": t, "d": d }));
        }
    }
}

/// Раскладка рейла серверов (папки + порядок отдельных гильдий) → единое
/// событие `discord://guild-layout` для UI. Источники: protobuf
/// `user_settings_proto` (современные аккаунты) или legacy-JSON
/// `guild_folders` (топ-уровень / user_settings.guild_folders).
fn emit_guild_layout(app: &tauri::AppHandle, d: &Value) {
    // 1. Прото: READY → d.user_settings_proto; PROTO_UPDATE → d.settings.proto.
    let b64 = ["user_settings_proto"]
        .iter()
        .find_map(|k| d.get(*k).and_then(|v| v.as_str()))
        .or_else(|| {
            d.get("settings")
                .and_then(|s| s.get("proto"))
                .and_then(|v| v.as_str())
        });
    if let Some(b64) = b64 {
        if let Some(layout) = crate::settings_proto::decode_layout(b64) {
            let mut value = layout_to_json(&layout);
            // rail_order прото-пути: отдельные гильдии (guild_positions),
            // затем папки (порядок внутри каждой сохранён).
            let mut order: Vec<Value> = layout
                .guild_positions
                .iter()
                .map(|gid| json!({ "type": "guild", "id": gid }))
                .collect();
            for (i, _) in layout.folders.iter().enumerate() {
                order.push(json!({ "type": "folder", "index": i }));
            }
            if let Some(obj) = value.as_object_mut() {
                obj.insert("rail_order".to_string(), Value::Array(order));
            }
            let _ = app.emit("discord://guild-layout", value);
            tracing::info!(
                "gateway: раскладка рейла (proto): {} папок, {} отдельных гильдий",
                layout.folders.len(),
                layout.guild_positions.len()
            );
            return;
        }
    }
    // 2. Legacy JSON: [{id, name, color, guild_ids}] — порядок массива и
    //    есть порядок рейла: одиночные записи (1 гильдия без id/name) —
    //    «просто серверы», остальные — папки. Классифицируем каждую запись
    //    и сохраняем точный interleaving в rail_order.
    let folders = d
        .get("guild_folders")
        .or_else(|| d.get("user_settings").and_then(|s| s.get("guild_folders")))
        .and_then(|v| v.as_array());
    if let Some(arr) = folders {
        let mut norm = Vec::new();
        let mut order: Vec<Value> = Vec::new();
        let mut positions: Vec<String> = Vec::new();
        for f in arr {
            let Some(ids) = f.get("guild_ids").and_then(|v| v.as_array()).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            }) else {
                continue;
            };
            let is_folder = ids.len() > 1
                || f.get("name").and_then(|v| v.as_str()).is_some()
                || f.get("id").and_then(|v| v.as_str()).is_some();
            if is_folder {
                order.push(json!({ "type": "folder", "index": norm.len() }));
                norm.push(json!({
                    "id": f.get("id").and_then(|v| v.as_str()),
                    "name": f.get("name").and_then(|v| v.as_str()),
                    "color": f.get("color").and_then(|v| v.as_i64()),
                    "guild_ids": ids,
                }));
            } else if let Some(gid) = ids.first() {
                // Одиночная запись: сервер сам по себе.
                order.push(json!({ "type": "guild", "id": gid }));
                positions.push(gid.clone());
            }
        }
        let _ = app.emit(
            "discord://guild-layout",
            json!({
                "folders": norm,
                "guild_positions": positions,
                "rail_order": order,
            }),
        );
        tracing::info!(
            "gateway: раскладка рейла (legacy): {} папок, {} отдельных гильдий",
            norm.len(),
            positions.len()
        );
    }
}

/// Прото-раскладка → JSON события (та же схема, что у legacy-пути).
fn layout_to_json(layout: &crate::settings_proto::GuildLayout) -> Value {
    let folders: Vec<Value> = layout
        .folders
        .iter()
        .map(|f| {
            json!({
                "id": f.id.map(|v| v.to_string()),
                "name": f.name.clone(),
                "color": f.color,
                "guild_ids": f.guild_ids,
            })
        })
        .collect();
    json!({
        "folders": folders,
        "guild_positions": layout.guild_positions,
    })
}

/// Пресенс из merged_presences: в отличие от обычных пресенсов
/// (встроенный user), эти приходят в формате {user_id, status,
/// activities} — дедупликация DEDUPE_USER_OBJECTS выносит юзера в
/// отдельный справочник. Строгий разбор Presence здесь молча проваливался
/// (user отсутствует → serde-ошибка → запись выбрасывалась) — статусы
/// друзей/гильдей из merged-массивов терялись целиком (v0.9.8).
fn merged_presence(v: &Value) -> Option<Presence> {
    let uid = v
        .get("user")
        .and_then(|u| u.get("id"))
        .and_then(|x| x.as_str())
        .or_else(|| v.get("user_id").and_then(|x| x.as_str()))?;
    if uid.is_empty() {
        return None;
    }
    let user = v
        .get("user")
        .and_then(|u| serde_json::from_value::<PresenceUser>(u.clone()).ok())
        .unwrap_or_default();
    let status = v
        .get("status")
        .and_then(|x| x.as_str())
        .unwrap_or("offline")
        .to_string();
    let activities = v
        .get("activities")
        .and_then(|x| serde_json::from_value(x.clone()).ok())
        .unwrap_or_default();
    Some(Presence {
        user: PresenceUser {
            id: uid.to_string(),
            ..user
        },
        guild_id: None,
        status,
        activities,
    })
}

/// Разбор merged-массивов READY / READY_SUPPLEMENTAL: `merged_members[i]` и
/// `merged_presences.guilds[i]` параллельны `d.guilds[i]`, а
/// `merged_presences.friends` — статусы друзей (лечит «все не в сети»).
/// Вызывается ВНЕ любых блокировок кэша.
fn apply_merged(state: &SharedState, d: &Value) {
    let Some(guilds) = d.get("guilds").and_then(|v| v.as_array()) else {
        return;
    };
    let guild_id_at = |i: usize| -> Option<String> {
        guilds
            .get(i)
            .and_then(|g| g.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
    };
    let mut friend_n = 0usize;
    let mut member_n = 0usize;
    let mut guild_pres_n = 0usize;
    {
        let mut cache = state.cache.write().unwrap();
        // merged_presences.friends → статусы друзей.
        if let Some(friends) = d
            .get("merged_presences")
            .and_then(|mp| mp.get("friends"))
            .and_then(|v| v.as_array())
        {
            for p in friends {
                if let Some(pr) = merged_presence(p) {
                    cache.friend_presences.insert(pr.user.id.clone(), pr);
                    friend_n += 1;
                }
            }
        }
        // merged_presences.guilds[i] ↔ guilds[i] — статусы по гильдиям.
        if let Some(gp) = d
            .get("merged_presences")
            .and_then(|mp| mp.get("guilds"))
            .and_then(|v| v.as_array())
        {
            for (i, arr) in gp.iter().enumerate() {
                let Some(gid) = guild_id_at(i) else { continue };
                let Some(gs) = cache.guilds.get_mut(&gid) else {
                    continue;
                };
                for p in arr.as_array().into_iter().flatten() {
                    if let Some(pr) = merged_presence(p) {
                        gs.presences.insert(pr.user.id.clone(), pr);
                        guild_pres_n += 1;
                    }
                }
            }
        }
        // merged_members[i] ↔ guilds[i] — своя карточка участника (роли, ник).
        if let Some(mm) = d.get("merged_members").and_then(|v| v.as_array()) {
            for (i, arr) in mm.iter().enumerate() {
                let Some(gid) = guild_id_at(i) else { continue };
                let Some(gs) = cache.guilds.get_mut(&gid) else {
                    continue;
                };
                for m in arr.as_array().into_iter().flatten() {
                    if let Ok(mem) = serde_json::from_value::<Member>(m.clone()) {
                        if let Some(uid) = mem.user.as_ref().map(|u| u.id.clone()) {
                            gs.members.insert(uid, mem);
                            member_n += 1;
                        }
                    }
                }
            }
        }
    }
    if friend_n + member_n + guild_pres_n > 0 {
        tracing::info!(
            "gateway: merged-данные: {friend_n} статусов друзей, {guild_pres_n} гильдейских, {member_n} карточек участников"
        );
    }
}

/// Переиграть подписки ленивых списков участников после (RE)подключения.
/// Сервер держит подписки op 37 только в рамках соединения: после обрыва
/// и повторного IDENTIFY они пусты. Официальный клиент на CONNECTION_OPEN
/// и CONNECTION_RESUMED делает flush ВСЕХ подписок (GuildSubscriptionsStore
/// → updateGuildSubscriptions) — повторяем для каналов, чьи списки у нас
/// уже были (guild_id → channel_id + ranges хранится в state.member_lists;
/// ranges — чтобы не терять докрученные скроллом сотни строк).
fn replay_member_list_subscriptions(state: &SharedState) {
    if state.is_bot.load(Ordering::SeqCst) {
        return;
    }
    /// (guild_id, channel_id, ranges) — что переиграть.
    type ReplayItem = (String, Option<String>, Vec<(u64, u64)>);
    let replay: Vec<ReplayItem> = {
        let lists = state.member_lists.lock().unwrap();
        lists
            .iter()
            .map(|(gid, l)| {
                let cid = if l.channel_id == "0" {
                    None
                } else {
                    Some(l.channel_id.clone())
                };
                let ranges = if l.ranges.is_empty() {
                    vec![(0, 99)]
                } else {
                    l.ranges.clone()
                };
                (gid.clone(), cid, ranges)
            })
            .collect()
    };
    if replay.is_empty() {
        return;
    }
    let tx = state.gateway_tx.read().unwrap().clone();
    if let Some(tx) = tx {
        tauri::async_runtime::spawn(async move {
            for (gid, cid, ranges) in replay {
                if tx
                    .send(crate::state::GatewayCommand::SubscribeMemberList {
                        guild_id: gid,
                        channel_id: cid,
                        ranges,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        tracing::info!("gateway: переигрываем подписки member-list после переподключения");
    }
}
