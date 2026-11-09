//! RustyCord protocol core — весь Discord-протокол без привязки к UI.
//!
//! Один и тот же крейт обслуживает два фронтенда:
//!  * `src-tauri` — Electron-подобное приложение (Tauri webview + Svelte);
//!  * `app-gpui` — нативный клиент на GPUI (Zed).
//!
//! Контракт с UI — два трейта:
//!  * [`EventSink`] — куда ядро отправляет события (`discord://…`),
//!    Tauri-фронтенд мостит их в `app.emit`, GPUI — в mpsc-канал;
//!  * [`auth::captcha::CaptchaSolver`] — решение Cloudflare Turnstile /
//!    hCaptcha в браузерном контексте (Tauri: WebviewWindow, GPUI: wry).

pub mod auth;
pub mod gateway;
pub mod rest;
pub mod session;
pub mod settings_proto;
pub mod snowflake;
pub mod state;
pub mod superprops;
pub mod updater;
pub mod voice;
pub mod wsutil;

use serde_json::Value;
use std::sync::Arc;

/// Версия протокольного ядра (совпадает с версией приложения-релиза).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Приёмник событий ядра. Семантика — как у `tauri::Emitter::emit`:
/// `(имя события, JSON-пейлоад)`. Реализация обязана быть неблокирующей
/// (быстрая отправка в канал / event loop) — тяжёлая обработка на стороне UI.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value);
}

/// Удобный алиас для передачи синка в спавнимые задачи.
pub type SharedSink = Arc<dyn EventSink>;
