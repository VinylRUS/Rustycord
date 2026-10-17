//! Voice coordinator на songbird: ждёт пару gateway-событий голосового
//! рукопожатия (VOICE_STATE_UPDATE + VOICE_SERVER_UPDATE) и на каждое
//! завершённое поднимает `songbird::Driver` — движок сам ведёт WS к
//! медиасерверу, UDP, шифрование и кодек. Мы отвечаем за события UI,
//! микрофон и микширование приёма.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use songbird::driver::{DecodeConfig, DecodeMode, Driver};
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use songbird::id::{ChannelId as SbChannelId, GuildId as SbGuildId, UserId as SbUserId};
use songbird::{Config, ConnectionInfo};
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::state::{SharedState, VoiceHandshake, VoiceSessionHandle};
use crate::voice::jitter::Mixer;
#[cfg(feature = "audio")]
use crate::voice::mic;
use crate::voice::VoiceControls;

#[cfg(feature = "audio")]
use crate::voice::devices;

pub async fn coordinator(app: tauri::AppHandle, state: SharedState, shutdown: CancellationToken) {
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // consume immediate tick

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = state.voice_notify.notified() => { poll_and_dial(&app, &state).await; }
            _ = ticker.tick() => { poll_and_dial(&app, &state).await; }
        }
    }
    tracing::info!("voice: координатор остановлен");
}

async fn poll_and_dial(app: &tauri::AppHandle, state: &SharedState) {
    // 1. Забираем все завершённые рукопожатия.
    let ready: Vec<VoiceHandshake> = {
        let mut map = state.voice_handshake.lock().unwrap();
        let done: Vec<String> = map
            .iter()
            .filter(|(_, hs)| hs.complete())
            .map(|(k, _)| k.clone())
            .collect();
        done.into_iter().filter_map(|k| map.remove(&k)).collect()
    };
    if ready.is_empty() {
        return;
    }

    for hs in ready {
        // 2. Гасим предыдущую сессию этой гильдии, если была.
        {
            let mut tasks = state.voice_tasks.lock().await;
            if let Some((handle, token)) = tasks.remove(&hs.guild_id) {
                token.cancel();
                handle.abort();
            }
        }
        // 3. Поднимаем новую.
        let token = CancellationToken::new();
        let guild_id = hs.guild_id.clone();
        let task = tauri::async_runtime::spawn(run_session(
            app.clone(),
            Arc::clone(state),
            hs,
            token.clone(),
        ));
        state
            .voice_tasks
            .lock()
            .await
            .insert(guild_id, (task, token));
    }
}

/// Одна голосовая сессия: Driver + устройства + события → UI.
async fn run_session(
    app: tauri::AppHandle,
    state: SharedState,
    hs: VoiceHandshake,
    cancel: CancellationToken,
) {
    let guild_id = hs.guild_id.clone();
    let kind = if hs.is_dm {
        "звонок ЛС"
    } else {
        "гильдия"
    };
    tracing::info!("voice[songbird]: сессия ({kind}) для {guild_id}");
    let res = voice_session(&app, &state, &hs, &cancel).await;
    if let Err(e) = res {
        tracing::error!("voice[songbird]: ошибка сессии: {e:#}");
        let _ = app.emit(
            "discord://voice",
            json!({ "kind": "disconnected", "data": { "guild_id": guild_id, "dm": hs.is_dm } }),
        );
    }
    // Self-cleanup so a fresh join can start immediately.
    state.voice_tasks.lock().await.remove(&hs.guild_id);
    state.voice_sessions.lock().unwrap().remove(&hs.guild_id);
    state.voice_desired.lock().unwrap().remove(&hs.guild_id);
    tracing::info!("voice[songbird]: сессия {guild_id} завершена");
}

async fn voice_session(
    app: &tauri::AppHandle,
    state: &SharedState,
    hs: &VoiceHandshake,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let Some(session_id) = hs.session_id.clone() else {
        anyhow::bail!("нет session_id");
    };
    let Some(token) = hs.token.clone() else {
        anyhow::bail!("нет voice token");
    };
    let Some(raw_endpoint) = hs.endpoint.clone() else {
        anyhow::bail!("нет endpoint");
    };
    let endpoint = raw_endpoint
        .trim()
        .trim_start_matches("wss://")
        .trim_end_matches('/')
        .to_string();
    let user_id: u64 = hs
        .user_id
        .parse()
        .map_err(|_| anyhow::anyhow!("user_id не парсится: {}", hs.user_id))?;
    let guild_num: u64 = hs
        .guild_id
        .parse()
        .map_err(|_| anyhow::anyhow!("guild_id не парсится: {}", hs.guild_id))?;
    let channel_num: u64 = hs
        .channel_id
        .as_deref()
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("в рукопожатии нет channel_id (гильдия {})", hs.guild_id))?;

    // Стартовые флаги мьюта — из последнего op4 (их записал join_voice),
    // дальше события-эхо в events.rs обновляют `controls` на лету.
    let (d_mute, d_deaf) = state
        .voice_desired
        .lock()
        .unwrap()
        .get(&hs.guild_id)
        .cloned()
        .unwrap_or((false, false));
    let controls = Arc::new(Mutex::new(VoiceControls::new(d_mute, d_deaf)));

    // ---- Приём: VoiceTick → Mixer (per-ssrc дорожки) ----
    let mixer = Arc::new(Mutex::new(Mixer::default()));
    // ssrc → user_id (заполняется из SpeakingStateUpdate) и seq-счётчики
    // для дорожек микшера (VoiceTick уже упорядочен, поэтому просто ++).
    let ssrc_map: Arc<Mutex<HashMap<u32, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let seqs: Arc<Mutex<HashMap<u32, u16>>> = Arc::new(Mutex::new(HashMap::new()));

    // ---- Микрофон (только с фичей audio) ----
    #[cfg(feature = "audio")]
    let mic_pair = mic::mic_channel();
    #[cfg(feature = "audio")]
    let tap = Arc::new(mic_pair.0);
    #[cfg(feature = "audio")]
    let stream = mic_pair.1;

    // Устройства: динамики (Mixer) + микрофон (MicTap → DSP). Guard держит
    // живые Stream'ы в отдельном std-потоке (cpal::Stream не Send на части
    // платформ) — ровно до конца сессии.
    #[cfg(feature = "audio")]
    let _dev_guard = devices::open(
        &mixer,
        &controls,
        &tap,
        &state.voice_dsp_cfg,
        &state.voice_dsp_metrics,
        &state.voice_dsp_active,
    );

    // ---- Driver ----
    // DecodeMode::Decode — songbird декодирует входящий opus в PCM
    // (VoiceTick.decoded_voice, стерео 48 кГц — родной формат Mixer);
    // без этого только speaking-события.
    let config = Config::default().decode_mode(DecodeMode::Decode(DecodeConfig::default()));
    let mut driver = Driver::new(config);

    // События движка → UI и микшер.
    driver.add_global_event(
        Event::Core(CoreEvent::VoiceTick),
        TickHandler {
            app: app.clone(),
            guild_id: hs.guild_id.clone(),
            mixer: Arc::clone(&mixer),
            ssrc_map: Arc::clone(&ssrc_map),
            seqs: Arc::clone(&seqs),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::SpeakingStateUpdate),
        SpeakHandler {
            app: app.clone(),
            guild_id: hs.guild_id.clone(),
            ssrc_map: Arc::clone(&ssrc_map),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::ClientDisconnect),
        DisconnectHandler {
            app: app.clone(),
            guild_id: hs.guild_id.clone(),
            ssrc_map: Arc::clone(&ssrc_map),
            mixer: Arc::clone(&mixer),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::DriverConnect),
        ConnectedHandler {
            app: app.clone(),
            guild_id: hs.guild_id.clone(),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::DriverDisconnect),
        DisconnectedHandler {
            app: app.clone(),
            guild_id: hs.guild_id.clone(),
        },
    );

    // Микрофон → трек songbird. play_only_input: один живой источник.
    #[cfg(feature = "audio")]
    let mic_track = if _dev_guard.has_input() {
        let input = mic::mic_input(stream);
        Some(driver.play_only_input(input))
    } else {
        tracing::warn!("voice[songbird]: микрофон недоступен — только приём");
        None
    };
    #[cfg(not(feature = "audio"))]
    let mic_track: Option<songbird::tracks::TrackHandle> = None;

    // Стартовый мьют (деафиен глушит микрофон, как в оригинале).
    driver.mute(d_mute || d_deaf);

    // Регистрируем сессию: events.rs по эху op4 обновит controls и driver.
    state.voice_sessions.lock().unwrap().insert(
        hs.guild_id.clone(),
        VoiceSessionHandle {
            controls: Arc::clone(&controls),
            driver: Some(driver.clone()),
            mic_track,
        },
    );

    // ---- Подключение ----
    let nz = |v: u64, what: &str| -> anyhow::Result<std::num::NonZeroU64> {
        std::num::NonZeroU64::new(v)
            .ok_or_else(|| anyhow::anyhow!("{what} = 0 (snowflake не бывает 0)"))
    };
    let info = ConnectionInfo {
        channel_id: SbChannelId::from(nz(channel_num, "channel_id")?),
        guild_id: SbGuildId::from(nz(guild_num, "guild_id")?),
        user_id: SbUserId::from(nz(user_id, "user_id")?),
        session_id,
        token,
        endpoint,
    };
    if let Err(e) = driver.connect(info).await {
        anyhow::bail!("songbird connect: {e}");
    }

    // ---- Жизнь сессии: до op4-выхода/замены/шатдауна ----
    cancel.cancelled().await;
    if let Some(handle) = state.voice_sessions.lock().unwrap().get(&hs.guild_id) {
        if let Some(t) = handle.mic_track.as_ref() {
            let _ = t.stop();
        }
    }
    Ok(())
}

/* --------------------------- handlers --------------------------- */

/// VoiceTick: декодированный PCM всех говорящих → микшер.
struct TickHandler {
    app: tauri::AppHandle,
    guild_id: String,
    mixer: Arc<Mutex<Mixer>>,
    ssrc_map: Arc<Mutex<HashMap<u32, String>>>,
    seqs: Arc<Mutex<HashMap<u32, u16>>>,
}

#[async_trait::async_trait]
impl EventHandler for TickHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::VoiceTick(tick) = ctx {
            let mut mixer = self.mixer.lock().unwrap();
            let mut seqs = self.seqs.lock().unwrap();
            for (ssrc, data) in &tick.speaking {
                let Some(pcm) = data.decoded_voice.as_ref() else {
                    continue;
                };
                // i16 stereo → f32 (родной формат Mixer).
                let frame: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
                let seq = *seqs.entry(*ssrc).or_insert(0);
                seqs.insert(*ssrc, seq.wrapping_add(1));
                mixer.on_frame(*ssrc, seq, frame);
                // Дебаг-трейс говорящего — только при первом кадре всплеска
                // не пишем (шумно); сам факт говорения едет из SPEAKING.
                let _ = &self.app;
                let _ = &self.guild_id;
            }
            let _ = &self.ssrc_map;
        }
        None
    }
}

/// SPEAKING (op 5 от медиасервера): ssrc ↔ user_id + событие UI.
struct SpeakHandler {
    app: tauri::AppHandle,
    guild_id: String,
    ssrc_map: Arc<Mutex<HashMap<u32, String>>>,
}

#[async_trait::async_trait]
impl EventHandler for SpeakHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::SpeakingStateUpdate(spk) = ctx {
            let speaking = spk
                .speaking
                .contains(songbird::model::SpeakingState::MICROPHONE);
            if let Some(uid) = spk.user_id {
                let uid = uid.0.to_string();
                self.ssrc_map.lock().unwrap().insert(spk.ssrc, uid.clone());
                let _ = self.app.emit(
                    "discord://voice",
                    json!({
                        "kind": "speaking",
                        "data": { "guild_id": self.guild_id, "user_id": uid, "speaking": speaking }
                    }),
                );
            }
        }
        None
    }
}

/// ClientDisconnect (op 13): убрать дорожку и погасить индикатор.
struct DisconnectHandler {
    app: tauri::AppHandle,
    guild_id: String,
    ssrc_map: Arc<Mutex<HashMap<u32, String>>>,
    mixer: Arc<Mutex<Mixer>>,
}

#[async_trait::async_trait]
impl EventHandler for DisconnectHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::ClientDisconnect(dc) = ctx {
            let uid = dc.user_id.0.to_string();
            let mut map = self.ssrc_map.lock().unwrap();
            if let Some(k) = map.iter().find(|(_, u)| **u == uid).map(|(k, _)| *k) {
                map.remove(&k);
                self.mixer.lock().unwrap().remove_channel(k);
            }
            let _ = self.app.emit(
                "discord://voice",
                json!({
                    "kind": "speaking",
                    "data": { "guild_id": self.guild_id, "user_id": uid, "speaking": false }
                }),
            );
        }
        None
    }
}

/// Успешное подключение к медиасерверу.
struct ConnectedHandler {
    app: tauri::AppHandle,
    guild_id: String,
}

#[async_trait::async_trait]
impl EventHandler for ConnectedHandler {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        tracing::info!("voice[songbird]: подключено к {}", self.guild_id);
        let _ = self.app.emit(
            "discord://voice",
            json!({
                "kind": "connected",
                "data": {
                    "guild_id": self.guild_id,
                    "mode": "aead_aes256_gcm (songbird)"
                }
            }),
        );
        None
    }
}

/// Разрыв соединения с медиасервером.
struct DisconnectedHandler {
    app: tauri::AppHandle,
    guild_id: String,
}

#[async_trait::async_trait]
impl EventHandler for DisconnectedHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        tracing::warn!("voice[songbird]: соединение потеряно ({:?})", ctx);
        let _ = self.app.emit(
            "discord://voice",
            json!({ "kind": "disconnected", "data": { "guild_id": self.guild_id } }),
        );
        None
    }
}
