//! The Discord Gateway client — a single WebSocket to wss://gateway.discord.gg
//! that carries every real-time event, driven by a small opcode machine:
//!
//! op 0 DISPATCH · op 1 HEARTBEAT · op 2 IDENTIFY · op 4 VOICE_STATE_UPDATE
//! op 6 RESUME · op 7 RECONNECT · op 8 REQUEST_GUILD_MEMBERS
//! op 9 INVALID_SESSION · op 10 HELLO · op 11 HEARTBEAT_ACK

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::gateway::events::{handle_dispatch, SessionData};
use crate::state::{GatewayCommand, SharedState, INTENTS_FALLBACK};
use crate::wsutil;

/// capabilities для юзер-сессии — битовая маска клиентских возможностей,
/// которую шлёт официальный веб-клиент (без интентов). Значение из
/// discord-client-rs: «лучшее, если нужны все события».
const USER_CAPABILITIES: u32 = 53_607_934;

/// Сколько последних исходящих кадров держим для дампа при close 4002.
const FRAME_LOG_CAP: usize = 10;
/// Лимит команд шлюза: 120 за скользящие 60 секунд (close 4008 сверх него).
const CMD_LIMIT: usize = 120;
const CMD_WINDOW: Duration = Duration::from_secs(60);

/* ---------- диагностика исходящих кадров (кольцевой буфер) ---------- */

/// Кольцевой буфер последних исходящих кадров (секреты скрыты). На close
/// 4002/4005 вываливается в лог одним блоком — «какой кадр сломал
/// декодирование» становится восстановимым и без verbose-логов. Пульсы
/// op 1 не пишутся (шум): они не бывают причиной 4002.
struct FrameLog {
    entries: VecDeque<String>,
    next_no: u64,
    started: std::time::Instant,
}

impl FrameLog {
    fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_no: 0,
            started: std::time::Instant::now(),
        }
    }

    /// Записать кадр: номер, возраст сессии, метка, размер, JSON без секретов.
    fn record(&mut self, label: &str, payload: &Value) {
        self.next_no = self.next_no.wrapping_add(1);
        let size = serde_json::to_vec(payload).map(|v| v.len()).unwrap_or(0);
        let mut body = redact(payload);
        if body.len() > 400 {
            body.truncate(400);
            body.push('…');
        }
        let age_ms = self.started.elapsed().as_millis();
        self.entries.push_back(format!(
            "  #{:<4} {:>7} мс  {:<26} {:>6} B  {}",
            self.next_no, age_ms, label, size, body
        ));
        while self.entries.len() > FRAME_LOG_CAP {
            self.entries.pop_front();
        }
    }

    /// Дамп буфера для лога/ошибки.
    fn dump(&self) -> String {
        let mut s =
            String::from("последние исходящие кадры (секреты скрыты; пульсы op 1 не пишутся):");
        for e in &self.entries {
            s.push('\n');
            s.push_str(e);
        }
        s
    }
}

/// Копия payload, в которой значения ключей `token`/`session_id` заменены
/// на `<redacted>` на любой глубине. Остальные кадры секретов не несут
/// (guild/channel id — не секрет в контексте локального лога).
fn redact(v: &Value) -> String {
    fn walk(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (k, val) in map {
                    if k == "token" || k == "session_id" {
                        out.insert(k.clone(), Value::String("<redacted>".into()));
                    } else {
                        out.insert(k.clone(), walk(val));
                    }
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(walk).collect()),
            other => other.clone(),
        }
    }
    walk(v).to_string()
}

/* ---------- лимит команд шлюза 120/60с ---------- */

/// Скользящее окно отправок: шлюз закрывает соединение (4008) сверх 120
/// команд за 60 секунд. Реальный трафик клиента далёк от лимита (десятки
/// в минуту при активном скролле) — бакет нужен как страховка от шторма
/// подписок, а не как ретрай-механика.
///
/// ВАЖНО (урок v0.9.4): занятие слота НЕ блокирует select-петлю. Урок
/// простой — async-пауза до 60 с внутри ветки rx.recv() замораживала
/// ВСЁ: входящие кадры не читались, heartbeat не отправлялся, шлюз
/// умирал «зомби» → реконнект → replay подписок → снова шторм. Теперь
/// при нехватке слотов команда уходит в отложенную очередь (см. run_session),
/// а петля продолжает читать сокет и слать пульсы.
struct CmdBucket {
    window: VecDeque<std::time::Instant>,
}

impl CmdBucket {
    fn new() -> Self {
        Self {
            window: VecDeque::new(),
        }
    }

    /// Выкинуть протухшие слоты, вернуть число свободных (синхронная часть
    /// — тестируется без рантайма).
    fn prune(&mut self, now: std::time::Instant) -> usize {
        while let Some(&t) = self.window.front() {
            if now.duration_since(t) >= CMD_WINDOW {
                self.window.pop_front();
            } else {
                break;
            }
        }
        CMD_LIMIT.saturating_sub(self.window.len())
    }

    /// Занять `n` слотов в момент `now`, если свободно достаточно.
    /// Атомарно: при нехватке не занимается ни один слот (команда —
    /// единое целое, частичная отправка мульти-кадрового bulk недопустима).
    fn try_acquire_at(&mut self, now: std::time::Instant, n: usize) -> bool {
        if n == 0 {
            return true;
        }
        if self.prune(now) >= n {
            for _ in 0..n {
                self.window.push_back(now);
            }
            true
        } else {
            false
        }
    }

    /// Синхронная обёртка для боевого кода.
    fn try_acquire(&mut self, n: usize) -> bool {
        self.try_acquire_at(std::time::Instant::now(), n)
    }

    /// Момент, когда освободится следующий слот (самый старый + окно).
    /// Пустое окно — «сейчас». До этого момента отложенные команды
    /// будить бессмысленно.
    fn next_free_at(&self) -> std::time::Instant {
        self.window
            .front()
            .map(|t| *t + CMD_WINDOW)
            .unwrap_or_else(std::time::Instant::now)
    }
}

/// Properties-набор как у браузерного клиента discord.com/app — совпадает
/// с нашим X-Super-Properties, чтобы сессия выглядела консистентно.
fn client_properties(build: u64) -> serde_json::Value {
    json!({
        "os": "Windows",
        "browser": "Chrome",
        "device": "",
        "system_locale": "ru-RU",
        "browser_user_agent": crate::superprops::BROWSER_UA,
        "browser_version": crate::superprops::BROWSER_VERSION,
        "os_version": "10",
        "referrer": "",
        "referring_domain": "",
        "referrer_current": "",
        "referring_domain_current": "",
        "release_channel": "stable",
        "client_build_number": build,
        "client_event_source": null,
        "design_id": 0
    })
}

/// How the current session ended.
enum End {
    /// Clean shutdown requested by the UI.
    Shutdown,
    /// Connection dropped / server asked to reconnect — try to RESUME.
    Reconnect,
    /// Сессию убил close 4002/4005 — сервер не смог декодировать наш кадр.
    /// Несёт дамп последних кадров; вызывающий считает подряд идущие.
    ProtocolError(String),
    /// Unrecoverable error (bad token, etc.) — stop the loop.
    Fatal(String),
}

/// Entry point: reconnect loop with exponential backoff (1s → 30s).
/// `session` survives across iterations so a RESUME picks up where we left.
pub async fn run_gateway(
    app: tauri::AppHandle,
    state: SharedState,
    mut rx: mpsc::Receiver<GatewayCommand>,
    shutdown: CancellationToken,
) {
    let mut session = SessionData::default();
    let mut backoff = 1u64;
    // Подряд идущие 4002/4005: детерминированно плохой кадр будет валить
    // каждую новую сессию — после трёх останавливаемся (серия частых
    // IDENTIFY на юзер-токене ещё и антифрод-риск), не молотя вечно.
    let mut protocol_errors = 0u32;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        if session.id.is_some() {
            state.emit_status("reconnecting", None);
        } else {
            state.emit_status("connecting", None);
        }
        let mut ready_seen = false;
        let end = run_session(
            &app,
            &state,
            &mut rx,
            &mut session,
            &shutdown,
            &mut ready_seen,
        )
        .await;
        if ready_seen {
            // Сессия дошла до READY и работала — обрыв не «серия»: и backoff,
            // и счётчик протокольных ошибок начинаем заново.
            backoff = 1;
            protocol_errors = 0;
        }
        match end {
            Ok(End::Shutdown) => break,
            Ok(End::Reconnect) => {
                tracing::info!(
                    "gateway: сессия прервана, возобновляем (resume={})",
                    session.id.is_some()
                );
            }
            Ok(End::ProtocolError(dump)) => {
                protocol_errors += 1;
                tracing::error!("gateway: close 4002/4005 (подряд: {protocol_errors})\n{dump}");
                if protocol_errors >= 3 {
                    let msg = "Шлюз трижды подряд отклонил наш кадр (close 4002). \
                               Сессия остановлена, чтобы не спамить IDENTIFY — \
                               детали последних кадров в лог-файле.";
                    tracing::error!("gateway: {msg}\n{dump}");
                    let _ = app.emit(
                        "discord://warning",
                        json!({ "message": format!("{msg} Путь к логу: Настройки → Логи.") }),
                    );
                    state.emit_status("disconnected", Some(msg.to_string()));
                    break;
                }
            }
            Ok(End::Fatal(msg)) => {
                tracing::error!("gateway: фатальная ошибка: {msg}");
                state.emit_status("disconnected", Some(msg));
                break;
            }
            Err(e) => {
                tracing::error!("gateway: ошибка сессии: {e:#}");
                state.emit_status("disconnected", Some(format!("{e:#}")));
            }
        }
        if shutdown.is_cancelled() {
            break;
        }
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
        }
        backoff = (backoff * 2).min(30);
    }
    state.emit_status("shutdown", None);
    tracing::info!("gateway: остановлен");
}

async fn run_session(
    app: &tauri::AppHandle,
    state: &SharedState,
    rx: &mut mpsc::Receiver<GatewayCommand>,
    session: &mut SessionData,
    shutdown: &CancellationToken,
    ready_seen: &mut bool,
) -> Result<End> {
    let base = state
        .gateway_url
        .read()
        .unwrap()
        .trim_end_matches('/')
        .to_string();
    // Версия протокола: юзер-сессии — v9 (ровно как официальный веб-клиент
    // и все self-hosted реализации; v10 — только бот-режим). RESUME обязан
    // идти на той же версии, что и исходный IDENTIFY.
    let ver = if state.is_bot.load(Ordering::SeqCst) {
        10
    } else {
        9
    };
    // READY hands us a dedicated resume_gateway_url — using it is the
    // documented (and reverse-engineered) way to resume safely.
    let url = match (&session.id, &session.resume_url) {
        (Some(_), Some(resume)) => format!("{resume}/?v={ver}&encoding=json"),
        _ => format!("{base}/?v={ver}&encoding=json"),
    };
    tracing::info!("gateway: подключение к {url}");

    let (ws, _) = connect_async(url)
        .await
        .context("не удалось открыть WebSocket до gateway.discord.gg")?;
    let (mut writer, mut reader) = wsutil::split(ws);
    let mut frames = FrameLog::new();
    let mut bucket = CmdBucket::new();

    // ---- HELLO (op 10): the server dictates our heartbeat cadence ----
    let hello = wsutil::next_json(&mut reader).await?;
    if hello["op"].as_u64() != Some(10) {
        bail!("ожидали HELLO (op 10), получили op {:?}", hello["op"]);
    }
    let hb_interval = hello["d"]["heartbeat_interval"]
        .as_u64()
        .unwrap_or(41_250)
        .clamp(1000, 300_000);
    tracing::info!("gateway: HELLO, heartbeat каждые {hb_interval} мс");

    // ---- IDENTIFY (op 2) or RESUME (op 6) ----
    let token = state
        .token
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow!("нет токена"))?;
    if let (Some(id), Some(seq)) = (&session.id, session.seq) {
        tracing::info!("gateway: RESUME сессии {id} с seq={seq}");
        let payload = json!({
            "op": 6,
            "d": { "token": token, "session_id": id, "seq": seq }
        });
        frames.record("RESUME (op 6)", &payload);
        wsutil::send_json(&mut writer, &payload).await?;
    } else {
        let is_bot = state.is_bot.load(Ordering::SeqCst);
        let build = state.build_number.load(Ordering::SeqCst);
        // Юзер-сессия: официальный клиент НЕ шлёт intents — вместо них
        // capabilities + client_state. С intents юзер-токен получает
        // урезанный поток событий (гильдии могут не приехать вовсе).
        // Бот-сессия: как раньше, интенты обязательны (v10).
        let d = if is_bot {
            tracing::info!(
                "gateway: IDENTIFY (бот) с intents={}",
                state.intents.load(Ordering::SeqCst)
            );
            json!({
                "token": token,
                "intents": state.intents.load(Ordering::SeqCst),
                "properties": client_properties(build),
                "presence": {
                    "status": "online",
                    "since": null,
                    "activities": [],
                    "afk": false
                },
                "compress": false
            })
        } else {
            tracing::info!(
                "gateway: IDENTIFY (юзер) capabilities={USER_CAPABILITIES} без интентов, presence=unknown"
            );
            json!({
                "token": token,
                "capabilities": USER_CAPABILITIES,
                "properties": client_properties(build),
                "presence": {
                    "status": "unknown",
                    "since": 0,
                    "activities": [],
                    "afk": false
                },
                "compress": false,
                "client_state": { "guild_versions": {} }
            })
        };
        frames.record("IDENTIFY (op 2)", &json!({ "op": 2, "d": d }));
        wsutil::send_json(&mut writer, &json!({ "op": 2, "d": d })).await?;
    }

    let mut heartbeat = tokio::time::interval(Duration::from_millis(hb_interval));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await; // consume the immediate first tick

    let mut acked = true;
    // Команды (op 37/8/…), пришедшие ДО первого DISPATCH (READY), нельзя
    // слать сразу: сервер отвечает 4002 «Error while decoding payload»
    // на op 37 до READY (замечено в логе v0.9.1: клик по серверу во время
    // переподключения). Буферизуем и выливаем после READY — так же
    // поступает официальный клиент (CONNECTION_OPEN → flush подписок).
    // NOTE: при обрыве до READY накопленное теряется — это компенсирует
    // replay подписок в events.rs (READY/RESUMED → flush из кэша).
    let mut ready = false;
    let mut pending_cmds: Vec<GatewayCommand> = Vec::new();
    // Отложенные команды: лимит 120/60с исчерпан. Ждут освобождения слотов
    // БЕЗ блокировки select-петли — пульсы и входящие кадры обрабатываются
    // как ни в чём не бывало. Подписки списков участников коалесятся по
    // ключу «гильдия:канал»: шторм скролла большого сервера (десятки
    // обновлений диапазонов в минуту) схлопывается до последнего набора.
    let mut deferred: VecDeque<GatewayCommand> = VecDeque::new();
    let mut wake = tokio::time::Instant::now();

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                let _ = writer.send(Message::Close(None)).await;
                return Ok(End::Shutdown);
            }

            cmd = rx.recv() => {
                let Some(cmd) = cmd else { return Ok(End::Shutdown) };
                if !ready {
                    // Сессия ещё не READY — копим и отправим после dispatch'а.
                    pending_cmds.push(cmd);
                    continue;
                }
                dispatch_cmd(app, &mut writer, cmd, &mut frames, &mut bucket,
                             &mut deferred, &mut wake).await?;
            }

            // Будильник отложенной очереди: как только старейший слот
            // протух — выливаем сколько влезает (обычно всё).
            _ = tokio::time::sleep_until(wake), if !deferred.is_empty() => {
                while !deferred.is_empty() {
                    let need = frames_needed(deferred.front().expect("deferred непуст"));
                    if !bucket.try_acquire(need) {
                        break;
                    }
                    let cmd = deferred.pop_front().expect("deferred непуст");
                    send_command(app, &mut writer, &cmd, &mut frames).await?;
                }
                if !deferred.is_empty() {
                    wake = tokio::time::Instant::from_std(bucket.next_free_at());
                }
            }
            _ = heartbeat.tick() => {
                if !acked {
                    // Пульс отправлен, ACK не пришёл → «зомби»-соединение
                    // (по документации достаточно одного пропущенного ACK).
                    tracing::warn!("gateway: нет HEARTBEAT_ACK — соединение протухло");
                    return Ok(End::Reconnect);
                }
                wsutil::send_json(&mut writer, &json!({ "op": 1, "d": session.seq })).await?;
                acked = false;
            }

            msg = reader.next() => {
                let Some(msg) = msg else {
                    tracing::warn!("gateway: поток закрылся");
                    return Ok(End::Reconnect);
                };
                let msg = msg.context("gateway ws ошибка")?;
                match msg {
                    Message::Text(text) => {
                        let v: Value = serde_json::from_str(&text).context("битый JSON кадра")?;
                        match v["op"].as_u64().unwrap_or(u64::MAX) {
                            0 => {
                                // DISPATCH: t = event name, s = sequence, d = payload.
                                // Разбор больше не фатален: кривое событие
                                // логируется внутри, но сессию НЕ убивает.
                                let t = v["t"].as_str().unwrap_or("").to_string();
                                if let Some(s) = v["s"].as_u64() {
                                    session.seq = Some(s);
                                }
                                let d = v["d"].clone();
                                handle_dispatch(app, state, &t, d, session).await;
                                // Первый dispatch — это READY (IDENTIFY-сессия):
                                // шлюз готов принимать команды. Выливаем всё,
                                // что накопилось за время рукопожатия.
                                if !ready {
                                    ready = true;
                                    *ready_seen = true;
                                    let queued = std::mem::take(&mut pending_cmds);
                                    if !queued.is_empty() {
                                        tracing::info!(
                                            "gateway: после READY отправляем {} отложенных команд",
                                            queued.len()
                                        );
                                        for cmd in queued {
                                            dispatch_cmd(app, &mut writer, cmd, &mut frames,
                                                         &mut bucket, &mut deferred, &mut wake).await?;
                                        }
                                    }
                                }
                            }
                            1 => {
                                // Сервер сам попросил пульс — отвечаем немедленно.
                                wsutil::send_json(&mut writer, &json!({ "op": 1, "d": session.seq })).await?;
                            }
                            7 => {
                                tracing::info!("gateway: сервер попросил RECONNECT");
                                return Ok(End::Reconnect);
                            }
                            9 => {
                                let resumable = v["d"].as_bool().unwrap_or(false);
                                tracing::warn!("gateway: INVALID_SESSION (resumable={resumable})");
                                if !resumable {
                                    session.id = None;
                                    session.resume_url = None;
                                    // Без session_id новый heartbeat обязан идти
                                    // с d=null, а не со seq прошлой сессии.
                                    session.seq = None;
                                }
                                // документированная пауза 1–5 c перед повтором
                                let jitter = 1000 + (std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH).unwrap()
                                    .subsec_millis() % 4000);
                                tokio::time::sleep(Duration::from_millis(jitter as u64)).await;
                                return Ok(End::Reconnect);
                            }
                            11 => { acked = true; }
                            other => tracing::debug!("gateway: игнорируем op {other}"),
                        }
                    }
                    Message::Close(Some(cf)) => {
                        let code = u16::from(cf.code);
                        tracing::warn!("gateway: закрыто сервером, код {code} ({})", cf.reason);
                        match code {
                            4004 => return Ok(End::Fatal("Токен отклонён (close 4004). Возможно, сессия сброшена — войди заново.".into())),
                            4010 | 4011 => return Ok(End::Fatal(format!("Неподдерживаемая конфигурация шардирования (close {code})."))),
                            4013 | 4014 => {
                                // Privileged intents выключены в Developer Portal.
                                state.intents.store(INTENTS_FALLBACK, Ordering::SeqCst);
                                session.id = None;
                                tracing::warn!("gateway: интенты запрещены (4014) — деградируем на fallback-набор");
                                let _ = app.emit(
                                    "discord://warning",
                                    json!({ "message": "Privileged Intents выключены в Developer Portal — перезапускаюсь без Presence/Members/MessageContent. Включи их для полной функциональности." }),
                                );
                                return Ok(End::Reconnect);
                            }
                            4002 | 4005 => {
                                // Декод нашего исходящего payload сломан /
                                // повторный IDENTIFY. Сессия недееспособна:
                                // RESUME на неё всегда ответит INVALID_SESSION —
                                // сразу чистим и идём на свежий IDENTIFY.
                                // Дамп последних кадров уходит в лог: какой
                                // кадр сломал декодирование, видно без
                                // verbose-режима (секреты скрыты в redact).
                                session.id = None;
                                session.resume_url = None;
                                session.seq = None;
                                state.emit_status(
                                    "reconnecting",
                                    Some(format!("close {code}: {}", cf.reason)),
                                );
                                return Ok(End::ProtocolError(frames.dump()));
                            }
                            4007 | 4009 => {
                                // Неверный seq при RESUME / сессия истекла на
                                // сервере — RESUME по этим кодам бесполезен,
                                // сразу свежий IDENTIFY (раньше попадали в
                                // ветку _ → лишний круг INVALID_SESSION).
                                session.id = None;
                                session.resume_url = None;
                                session.seq = None;
                                state.emit_status(
                                    "reconnecting",
                                    Some(format!("close {code}: {}", cf.reason)),
                                );
                                return Ok(End::Reconnect);
                            }
                            // Причина обрыва — в detail статуса: фронтенд показывает
                            // её в баннере, чтобы не гадать «почему отвалилось».
                            _ => {
                                state.emit_status(
                                    "reconnecting",
                                    Some(format!("close {code}: {}", cf.reason)),
                                );
                                return Ok(End::Reconnect);
                            }
                        }
                    }
                    Message::Close(None) => return Ok(End::Reconnect),
                    _ => {}
                }
            }
        }
    }
}

/// Чанкование bulk op 37 по JSON-весу: официальный клиент
/// (updateGuildSubscriptions) не кладёт в один кадр больше ~15 360 байт
/// подписок — иначе живой шлюз отвечает 4002. Возвращает ГОТОВЫЕ кадры.
fn build_bulk_frames(guild_ids: &[String]) -> Vec<Value> {
    let mut out_frames: Vec<Value> = Vec::new();
    let mut subs = serde_json::Map::new();
    let mut budget = 0usize;
    for gid in guild_ids {
        let sub = json!({
            "typing": true, "threads": true,
            "activities": true, "member_updates": true
        });
        let cost = format!("[{gid:?},{sub}]").len();
        if budget + cost > 15_360 {
            out_frames.push(json!({ "op": 37, "d": { "subscriptions": subs } }));
            subs = serde_json::Map::new();
            budget = 0;
        }
        subs.insert(gid.clone(), sub);
        budget += cost;
    }
    if !subs.is_empty() {
        out_frames.push(json!({ "op": 37, "d": { "subscriptions": subs } }));
    }
    out_frames
}

/// Сколько кадров порождает команда — столько слотов лимита она занимает.
/// Мульти-кадровая только bulk op 37 (чанки по 15 360 байт); всё остальное —
/// один кадр. Считается ДО отправки: слоты занимаются атомарно, частичная
/// отправка мульти-кадровой команды недопустима (хвост без головы —
/// гарантированный 4002).
fn frames_needed(cmd: &GatewayCommand) -> usize {
    match cmd {
        GatewayCommand::BulkGuildSubscribe { guild_ids } => build_bulk_frames(guild_ids).len(),
        GatewayCommand::SendText(_)
        | GatewayCommand::RequestMembers { .. }
        | GatewayCommand::SubscribeMemberList { .. }
        | GatewayCommand::VoiceStateUpdate { .. }
        | GatewayCommand::StreamCreate { .. }
        | GatewayCommand::StreamDelete { .. }
        | GatewayCommand::StreamSetPaused { .. } => 1,
    }
}

/// Потолок отложенной очереди. Очередь — страховка от шторма, а не склад:
/// при правильном трафике она живёт секунды. Переполнение означает
/// патологию (фронт спамит командами) — роняем самые старые, newest-wins.
const DEFERRED_CAP: usize = 256;

/// Положить команду в отложенную очередь с коалесингом: подписка списка
/// участников с тем же ключом «гильдия:канал» ЗАМЕНЯЕТ старую (шторм
/// скролла схлопывается до последнего набора диапазонов — ровно как
/// GUILD_SUBSCRIPTIONS_CHANNEL у официала перезаписывает предыдущую).
fn defer_with_coalesce(deferred: &mut VecDeque<GatewayCommand>, cmd: GatewayCommand) {
    if let GatewayCommand::SubscribeMemberList {
        guild_id,
        channel_id,
        ..
    } = &cmd
    {
        deferred.retain(|c| match c {
            GatewayCommand::SubscribeMemberList {
                guild_id: g,
                channel_id: ch,
                ..
            } => (g, ch) != (guild_id, channel_id),
            _ => true,
        });
    }
    if deferred.len() >= DEFERRED_CAP {
        let dropped = deferred.pop_front();
        if let Some(d) = dropped {
            tracing::warn!("gateway: отложенная очередь переполнена — выбрасываю {d:?}");
        }
    }
    deferred.push_back(cmd);
}

/// Развилка команды: есть слоты — отправить сейчас; нет — отложить,
/// НЕ блокируя select-петлю (урок v0.9.4: sleep внутри ветки замораживал
/// чтение кадров и heartbeat → зомби-соединение). Будильник очереди
/// переставляется на момент освобождения старейшего слота.
async fn dispatch_cmd(
    app: &tauri::AppHandle,
    writer: &mut crate::wsutil::WsWriter,
    cmd: GatewayCommand,
    frames: &mut FrameLog,
    bucket: &mut CmdBucket,
    deferred: &mut VecDeque<GatewayCommand>,
    wake: &mut tokio::time::Instant,
) -> Result<()> {
    let need = frames_needed(&cmd);
    // Патология: команда сама по себе больше лимита (bulk на тысячи гильдий).
    // Ждать слотов бессмысленно — их не наберётся никогда. Отправляем как
    // есть и ругаемся: лучше рискнуть 4008, чем заморозить очередь навсегда.
    if need < CMD_LIMIT && bucket.try_acquire(need) {
        send_command(app, writer, &cmd, frames).await
    } else {
        if need >= CMD_LIMIT {
            tracing::warn!("gateway: команда требует {need} кадров (лимит {CMD_LIMIT}) — отправляю без очереди");
            return send_command(app, writer, &cmd, frames).await;
        }
        tracing::warn!(
            "gateway: лимит {CMD_LIMIT}/60с исчерпан — команда в очередь ({} шт.)",
            deferred.len() + 1
        );
        defer_with_coalesce(deferred, cmd);
        *wake = tokio::time::Instant::from_std(bucket.next_free_at());
        Ok(())
    }
}

/// Отправка одного gateway-команды как кадра JSON. Вынесено из select-петли,
/// чтобы один и тот же код обслуживал и live-команды, и отложенные
/// (накопленные до READY и ждущие слотов лимита). Каждый кадр попадает
/// в кольцевой буфер (дамп при 4002); слоты лимита занимается вызывающий
/// (dispatch_cmd / будильник отложенной очереди) — по числу кадров команды.
async fn send_command(
    app: &tauri::AppHandle,
    writer: &mut crate::wsutil::WsWriter,
    cmd: &GatewayCommand,
    frames: &mut FrameLog,
) -> Result<()> {
    /// Записать кадр в буфер и отправить.
    async fn send_frame(
        writer: &mut crate::wsutil::WsWriter,
        frames: &mut FrameLog,
        label: &str,
        payload: &Value,
    ) -> Result<()> {
        frames.record(label, payload);
        wsutil::send_json(writer, payload).await
    }
    match cmd {
        GatewayCommand::SendText(text) => {
            // Консоль протокола: сырой текст. Невалидный JSON — верный
            // способ получить 4002 и убить сессию, поэтому валидируем
            // ДО отправки и ругаемся в UI, не трогая соединение.
            match serde_json::from_str::<Value>(text) {
                Ok(v) => {
                    send_frame(writer, frames, "SendText (консоль)", &v).await?;
                }
                Err(e) => {
                    tracing::warn!("gateway: SendText отклонён — не JSON: {e}");
                    let _ = app.emit(
                        "discord://warning",
                        json!({ "message": format!("Протокол-консоль: кадр не отправлен — это не JSON ({e}).") }),
                    );
                }
            }
        }
        GatewayCommand::RequestMembers { guild_id } => {
            // op 8: query="" + limit=0 → the server streams every member in chunks
            send_frame(
                writer,
                frames,
                "op 8 RequestMembers",
                &json!({
                    "op": 8,
                    "d": { "guild_id": guild_id, "query": "", "limit": 0 }
                }),
            )
            .await?;
        }
        GatewayCommand::BulkGuildSubscribe { guild_ids } => {
            // op 37 (юзер-сессии): подписка на события всех гильдий
            // сразу — presences/typing/threads/member_updates.
            // Чанкование как в официальном клиенте
            // (updateGuildSubscriptions): сумма JSON-весов пар
            // [gid, sub] не должна превышать 15 360 байт на кадр.
            let out_frames = build_bulk_frames(guild_ids);
            let n_frames = out_frames.len();
            for (i, frame) in out_frames.iter().enumerate() {
                send_frame(
                    writer,
                    frames,
                    &format!("op 37 bulk [{}/{}]", i + 1, n_frames),
                    frame,
                )
                .await?;
            }
            tracing::info!(
                "gateway: op 37 bulk subscribe на {} гильдий ({} кадров)",
                guild_ids.len(),
                n_frames
            );
        }
        GatewayCommand::SubscribeMemberList {
            guild_id,
            channel_id,
            ranges,
        } => {
            // op 37 на ОДНУ гильдию с ОДНИМ каналом — 1:1 с официальным
            // клиентом (subscribeChannel → {channels:{cid:ranges}}).
            // Каналов не больше 5 на гильдию (LRU ChannelsStore в бандле
            // discord.com) — больше живой шлюз закрывает 4002; v0.9.1 слал
            // 25 каналов и ловил 4002 на каждый клик по серверу.
            // channel_id = None → псевдоканал "0": общий список участников
            // гильдии (member sidebar без выбранного канала).
            // Диапазоны приходят от фронтенда: стартово [[0,99]], при
            // скролле — [[0,99],[100,199],…] до видимой зоны + overscan
            // (onScroll → GUILD_SUBSCRIPTIONS_CHANNEL у официала; всегда
            // включает [0,99], сотни выровнены по JM=100).
            let cid = channel_id.clone().unwrap_or_else(|| "0".to_string());
            let ranges: Vec<(u64, u64)> = if ranges.is_empty() {
                vec![(0, 99)]
            } else {
                ranges.clone()
            };
            let mut chans = serde_json::Map::new();
            // Vec<(u64,u64)> сериализуется как [[a,b],…] — как в бандле.
            chans.insert(cid.clone(), serde_json::to_value(&ranges)?);
            // Полный формат подписки — 1:1 с G.get() официального клиента
            // (путь replay на CONNECTION_OPEN шлёт ВСЕ поля, включая пустые
            // members/thread_member_lists — серверная схема строгая и при
            // наличии channels ждёт их; без них живой шлюз отвечал 4002).
            let mut subs = serde_json::Map::new();
            subs.insert(
                guild_id.clone(),
                json!({
                    "typing": true,
                    "threads": true,
                    "activities": true,
                    "member_updates": true,
                    "members": [],
                    "thread_member_lists": [],
                    "channels": chans
                }),
            );
            let payload = json!({
                "op": 37,
                "d": { "subscriptions": subs }
            });
            send_frame(writer, frames, "op 37 member-list", &payload).await?;
            let last = ranges.last().map(|(_, b)| b + 1).unwrap_or(0);
            tracing::info!(
                "gateway: op 37 список участников {guild_id} · канал {cid} · {} диапазон(ов), строк до {last}",
                ranges.len()
            );
        }
        GatewayCommand::VoiceStateUpdate {
            guild_id,
            channel_id,
            self_mute,
            self_deaf,
            self_video,
        } => {
            // op 4: войти/покинуть голосовой канал. Для звонков в ЛС/группе
            // guild_id = null — так шлёт официальный клиент (voice-сервер
            // идентифицирует сессию по channel_id).
            send_frame(
                writer,
                frames,
                "op 4 VoiceStateUpdate",
                &json!({
                    "op": 4,
                    "d": {
                        "guild_id": guild_id,     // null → звонок в ЛС
                        "channel_id": channel_id, // null → покинуть канал
                        "self_mute": self_mute,
                        "self_deaf": self_deaf,
                        "self_video": self_video
                    }
                }),
            )
            .await?;
        }
        GatewayCommand::StreamCreate {
            stream_type,
            guild_id,
            channel_id,
        } => {
            // op 18 STREAM_CREATE — «показать экран»/«включить камеру».
            // Сервер разошлёт dispatch STREAM_CREATE с stream_key — по нему
            // останавливаем (op 20) и ставим на паузу (op 22).
            send_frame(
                writer,
                frames,
                "op 18 StreamCreate",
                &json!({
                    "op": 18,
                    "d": {
                        "type": stream_type,
                        "guild_id": guild_id,
                        "channel_id": channel_id,
                        "preferred_region": null
                    }
                }),
            )
            .await?;
            tracing::info!("gateway: op 18 STREAM_CREATE отправлен");
        }
        GatewayCommand::StreamDelete { stream_key } => {
            send_frame(
                writer,
                frames,
                "op 20 StreamDelete",
                &json!({ "op": 20, "d": { "stream_key": stream_key } }),
            )
            .await?;
            tracing::info!("gateway: op 20 STREAM_DELETE {stream_key}");
        }
        GatewayCommand::StreamSetPaused { stream_key, paused } => {
            send_frame(
                writer,
                frames,
                "op 22 StreamSetPaused",
                &json!({ "op": 22, "d": { "stream_key": stream_key, "paused": paused } }),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_hides_secrets_at_any_depth() {
        let frame = json!({
            "op": 2,
            "d": {
                "token": "MTIzNDU2Nzg5MEdFVVMK",
                "session_id": "abc",
                "properties": { "browser": "Chrome", "nested": { "session_id": "deep" } },
                "list": [ { "token": "in-array" }, { "ok": 1 } ]
            }
        });
        let out = redact(&frame);
        assert!(!out.contains("MTIzNDU2Nzg5MEdFVVMK"));
        assert!(!out.contains("\"abc\""));
        assert!(!out.contains("deep"));
        assert!(!out.contains("in-array"));
        assert!(out.contains("<redacted>"));
        assert!(out.contains("Chrome")); // не-секреты не трогаем
        assert!(out.contains("\"ok\":1"));
    }

    #[test]
    fn frame_log_keeps_cap_and_formats() {
        let mut fl = FrameLog::new();
        for i in 0..(FRAME_LOG_CAP + 5) {
            fl.record(&format!("op {i}"), &json!({ "op": i }));
        }
        let dump = fl.dump();
        assert!(dump.lines().count() >= FRAME_LOG_CAP); // заголовок + кадры
                                                        // Самый старый кадр вытеснен, самый новый на месте.
        assert!(!dump.contains("#1 "));
        assert!(dump.contains(&format!("#{}", FRAME_LOG_CAP + 5)));
    }

    #[test]
    fn cmd_bucket_window_frees_with_age() {
        let mut b = CmdBucket::new();
        let now = std::time::Instant::now();
        // 120 свежих слотов — свободных нет.
        for _ in 0..CMD_LIMIT {
            b.window.push_back(now);
        }
        assert_eq!(b.prune(now), 0);
        // Через 61с все слоты протухли — окно свободно.
        let later = now + CMD_WINDOW + Duration::from_secs(1);
        assert_eq!(b.prune(later), CMD_LIMIT);
        assert!(b.window.is_empty());
        // Частичное старение: первая половина старая, вторая свежая
        // (реальные метки всегда хронологичны — окно отсортировано).
        for _ in 0..CMD_LIMIT / 2 {
            b.window.push_back(now); // старые
        }
        for _ in 0..CMD_LIMIT / 2 {
            b.window.push_back(later); // свежие
        }
        assert_eq!(b.prune(later), CMD_LIMIT / 2);
    }

    #[test]
    fn try_acquire_is_atomic_no_partial_take() {
        let mut b = CmdBucket::new();
        let now = std::time::Instant::now();
        // Свободно 119 слотов из 120 — команда на 3 кадра НЕ влезает…
        for _ in 0..(CMD_LIMIT - 1) {
            b.window.push_back(now);
        }
        assert!(!b.try_acquire_at(now, 3));
        // …и не занимает НИ ОДНОГО (частичная отправка bulk недопустима).
        assert_eq!(b.window.len(), CMD_LIMIT - 1);
        // Один кадр — влезает и занимает ровно один слот.
        assert!(b.try_acquire_at(now, 1));
        assert_eq!(b.window.len(), CMD_LIMIT);
        // Ноль кадров (пустая команда) — всегда ок, слоты не трогает.
        assert!(b.try_acquire_at(now, 0));
        assert_eq!(b.window.len(), CMD_LIMIT);
    }

    #[test]
    fn frames_needed_bulk_grows_with_guild_count() {
        // Одна гильдия — один кадр.
        let one = vec!["123".to_string()];
        assert_eq!(
            frames_needed(&GatewayCommand::BulkGuildSubscribe { guild_ids: one }),
            1
        );
        // Сотни гильдий — кадры по весу: чанкование обязательно.
        let many: Vec<String> = (0..500).map(|i| i.to_string()).collect();
        let n = frames_needed(&GatewayCommand::BulkGuildSubscribe { guild_ids: many });
        assert!(n > 1, "500 гильдий обязаны чанковаться, кадров {n}");
        // Все остальные команды — ровно один кадр.
        assert_eq!(
            frames_needed(&GatewayCommand::SubscribeMemberList {
                guild_id: "1".into(),
                channel_id: None,
                ranges: vec![(0, 99), (100, 199)],
            }),
            1
        );
    }

    #[test]
    fn deferred_coalesces_member_list_by_guild_channel() {
        let mut q = VecDeque::new();
        let mk = |ranges: Vec<(u64, u64)>| GatewayCommand::SubscribeMemberList {
            guild_id: "g1".into(),
            channel_id: Some("c1".into()),
            ranges,
        };
        defer_with_coalesce(&mut q, mk(vec![(0, 99)]));
        defer_with_coalesce(&mut q, mk(vec![(0, 99), (100, 199)]));
        defer_with_coalesce(
            &mut q,
            GatewayCommand::SubscribeMemberList {
                guild_id: "g1".into(),
                channel_id: Some("c2".into()),
                ranges: vec![(0, 99)],
            },
        );
        // c1-подписки схлопнулись (осталась последняя), c2 — отдельный ключ.
        assert_eq!(q.len(), 2);
        let c1 = q.iter().find(|c| {
            matches!(c,
            GatewayCommand::SubscribeMemberList { channel_id: Some(cid), .. } if cid == "c1")
        });
        assert!(
            matches!(c1, Some(GatewayCommand::SubscribeMemberList { ranges, .. })
            if ranges.len() == 2)
        );
    }
}
