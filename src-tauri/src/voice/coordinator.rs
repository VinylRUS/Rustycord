//! Voice coordinator: waits until the two gateway events that make up a
//! voice handshake (VOICE_STATE_UPDATE + VOICE_SERVER_UPDATE) have both
//! arrived for a guild, then dials the media server.
//!
//! The Notify primitive can coalesce wake-ups, so a 2s ticker acts as a
//! safety net — belt and braces, like every good real-time system.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::state::{SharedState, VoiceHandshake};
use crate::voice::ws;

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
        let task = tauri::async_runtime::spawn(ws::run_voice(
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
