//! Совместная оркестрация сессии для обоих фронтендов (Tauri и GPUI):
//! подключение по токену, останов задач, REST-фасад, QR-сессия логина.
//! Фронтенд-специфичного кода здесь нет — только протокол и задачи.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::json;

use crate::rest::client::RestClient;
use crate::rest::models::CurrentUser;
use crate::state::{AppState, INTENTS_BOT};
use crate::superprops;
use crate::voice::coordinator;
use crate::SharedSink;

impl AppState {
    /// REST-клиент текущей сессии (токен + свежий build number).
    /// Юзер-сессии ходят на v9 (как официальный веб-клиент; v10 отдаёт
    /// юзер-токенам 403 на ресурсных эндпоинтах), боты — на v10.
    pub fn rest(&self) -> Result<RestClient, String> {
        let token = self
            .token
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| "Не подключено — войди в аккаунт".to_string())?;
        let build = self.build_number.load(Ordering::SeqCst);
        let version = if self.is_bot.load(Ordering::SeqCst) { 10 } else { 9 };
        RestClient::new(&token, build, version).map_err(|e| e.to_string())
    }

    /// Остановить задачи gateway/voice/QR (если запущены).
    pub async fn stop_tasks(&self) {
        if let Some(task) = self.gateway_task.lock().await.take() {
            task.abort();
        }
        if let Some(task) = self.qr_task.lock().await.take() {
            task.abort();
        }
        self.shutdown.read().unwrap().cancel();
        *self.gateway_tx.write().unwrap() = None;
        let mut voices = self.voice_tasks.lock().await;
        for (_, (handle, token)) in voices.drain() {
            token.cancel();
            handle.abort();
        }
        self.voice_handshake.lock().unwrap().clear();
    }
}

/// Подключиться по токену: валидация, сброс кэша, запуск gateway + voice.
/// Возвращает профиль пользователя. Оба фронтенда зовут это вместо того,
/// чтобы дублировать порядок шагов (v0.9.x учил: порядок критичен —
/// подписки op 37 рассылаются из events.rs по READY).
pub async fn connect(state: &Arc<AppState>, token: &str) -> Result<CurrentUser, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Пустой токен".into());
    }

    state.stop_tasks().await;

    // Fail fast на мёртвый токен/сеть до спавна чего-либо.
    // v10 на /users/@me работает и юзеру, и боту (проверено живыми логами) —
    // первичный зонд логина остаётся на v10; последующие вызовы идут через
    // AppState::rest() (юзер → v9, бот → v10).
    let build = state.build_number.load(Ordering::SeqCst);
    let rest = RestClient::new(&token, build, 10).map_err(|e| e.to_string())?;
    let me: CurrentUser = rest.get_json("/users/@me").await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("401") {
            "Токен отклонён (401) — сессия могла быть сброшена. Войди заново через QR или email."
                .to_string()
        } else {
            msg
        }
    })?;

    // Свежий client_build_number в фоне — следующие REST-вызовы возьмут его
    // из state. /users/@me работает и со старым, так что логин не ждёт.
    {
        let st = Arc::clone(state);
        tokio::spawn(async move {
            if let Some(b) = superprops::fetch_build_number().await {
                st.build_number.store(b, Ordering::SeqCst);
            }
        });
    }

    // Fresh state.
    state.cache.write().unwrap().reset();
    state.cache.write().unwrap().user = Some(me.clone());
    *state.token.write().unwrap() = Some(token);
    // Подписки op 37/14 — заново для новой сессии.
    state.subscribed_guilds.lock().unwrap().clear();
    // Бот или юзер? Юзеры шлют capabilities без intents (как официальный
    // клиент) — иначе гильдии и события могут не приехать. Интенты — ботам.
    state.is_bot.store(me.bot, Ordering::SeqCst);
    state.intents.store(INTENTS_BOT, Ordering::SeqCst);
    // Пользовательская сессия ходит на общий gateway (без /gateway/bot,
    // это бот-эндпоинт — юзеру он вернёт 401).
    *state.gateway_url.write().unwrap() = "wss://gateway.discord.gg".to_string();
    if me.bot {
        tracing::info!("connect: бот @{} (id {}) — gateway с intents", me.username, me.id);
    } else {
        tracing::info!(
            "connect: пользователь @{} (id {}) — gateway с capabilities",
            me.username,
            me.id
        );
    }

    // Gateway task.
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    *state.gateway_tx.write().unwrap() = Some(tx);
    let shutdown = tokio_util::sync::CancellationToken::new();
    *state.shutdown.write().unwrap() = shutdown.clone();

    let sink: SharedSink = Arc::clone(&state.sink);
    let task = tokio::spawn(crate::gateway::client::run_gateway(
        sink,
        Arc::clone(state),
        rx,
        shutdown.clone(),
    ));
    *state.gateway_task.lock().await = Some(task);

    // Voice coordinator (idempotent, умирает с тем же shutdown-токеном).
    let sink = Arc::clone(&state.sink);
    tokio::spawn(coordinator::coordinator(sink, Arc::clone(state), shutdown));

    Ok(me)
}

/// Отключиться: останов задач, сброс кэша и токена.
pub async fn disconnect(state: &Arc<AppState>) {
    state.stop_tasks().await;
    state.cache.write().unwrap().reset();
    *state.token.write().unwrap() = None;
    state.emit_status("shutdown", None);
}

/* ------------------------- QR remote-auth ------------------------- */

/// Запустить QR-сессию логина (события стадий — `discord://qr`).
pub async fn start_remote_auth(state: &Arc<AppState>) -> Result<(), String> {
    let _ = cancel_remote_auth(state).await;
    let cancel = tokio_util::sync::CancellationToken::new();
    *state.qr_cancel.write().unwrap() = Some(cancel.clone());
    let sink = Arc::clone(&state.sink);
    let solver = Arc::clone(&state.captcha);
    let task = tokio::spawn(async move {
        if let Err(e) = crate::auth::remote::run_remote_auth(sink.clone(), solver, cancel).await {
            tracing::warn!("remote-auth: {e:#}");
            // Ошибка обязана попасть в UI — иначе QR просто «не появится».
            sink.emit(
                "discord://qr",
                json!({ "stage": "error", "message": format!("{e:#}") }),
            );
        }
    });
    *state.qr_task.lock().await = Some(task);
    Ok(())
}

/// Отменить QR-сессию (пользователь ушёл с таба/закрыл экран логина).
pub async fn cancel_remote_auth(state: &Arc<AppState>) -> Result<(), String> {
    if let Some(cancel) = state.qr_cancel.write().unwrap().take() {
        cancel.cancel();
    }
    // Даём задаче ~мгновение на вежливый close-кадр перед abort.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    if let Some(task) = state.qr_task.lock().await.take() {
        task.abort();
    }
    Ok(())
}
