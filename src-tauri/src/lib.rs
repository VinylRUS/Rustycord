//! ⚠️ **УСТАРЕЛО с версии 0.12.0** ⚠️
//! 
//! Tauri-фронтенд заменён нативным GPUI-клиентом (`app-gpui/`).
//! Этот код сохранён для справки и совместимости сборок, но новые фичи
//! добавляются только в GPUI-версию. Пользователям рекомендуется
//! переходить на `rustycord-gpui` (основной клиент).
//!
//! **Legacy-функционал:**
//! Вся логика протокола — в крейте `rustycord-core` (общий для обоих фронтов),
//! здесь только обвязка IPC (commands.rs), окно капчи (captcha_webview.rs)
//! и мостик событий TauriSink.

mod captcha_webview;
mod commands;

use std::sync::Arc;

use rustycord_core::state::AppState;
use rustycord_core::EventSink;
use serde_json::Value;
use tauri::Manager;

/// Мост событий ядра → Tauri: `EventSink::emit` → `tauri::Emitter::emit`
/// (фронтенд слушает через `listen()` из @tauri-apps/api).
struct TauriSink(tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: Value) {
        use tauri::Emitter;
        let _ = self.0.emit(event, payload);
    }
}

/// Куда пишем файловый лог (инициализируется в setup).
static LOG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Путь к активному файлу логов (для экрана настроек / баг-репортов).
pub fn log_file_path() -> String {
    LOG_PATH
        .get()
        .cloned()
        .unwrap_or_else(|| "логи недоступны".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(Arc::new(AppState::new(
                Arc::new(TauriSink(handle.clone())),
                Arc::new(captcha_webview::TauriCaptcha(handle.clone())),
            )));

            // ---- Логирование: stdout + суточный файл в app_log_dir ----
            // Релизная сборка на Windows не имеет консоли — без файла логов
            // диагностика «почему отвалился gateway» превращается в гадание.
            let mut guard_opt = None;
            let mut path_str = String::new();
            if let Ok(dir) = app.path().app_log_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let appender = tracing_appender::rolling::daily(&dir, "rustycord.log");
                let (non_blocking, guard) = tracing_appender::non_blocking(appender);
                guard_opt = Some(guard);
                // rolling::daily кладёт файл с датой; покажем каталог.
                path_str = dir
                    .join("rustycord.log.YYYY-MM-DD")
                    .to_string_lossy()
                    .to_string();
                let writer = non_blocking;
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| "rustycord=info".into()),
                    )
                    .with_ansi(false)
                    .with_writer(writer)
                    .try_init();
            } else {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| "rustycord=info".into()),
                    )
                    .try_init();
            }
            let _ = LOG_PATH.set(path_str);
            // guard должен жить всю сессию — отдать его leak'ом в static,
            // иначе NonBlockingWorker остановится при дропе.
            if let Some(g) = guard_opt {
                std::mem::forget(g);
            }

            tracing::info!(
                "RustyCord core is up (Tauri {}, protocol {})",
                tauri::VERSION,
                rustycord_core::VERSION
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::disconnect,
            commands::get_state,
            commands::get_messages,
            commands::send_message,
            commands::edit_message,
            commands::delete_message,
            commands::add_reaction,
            commands::remove_reaction,
            commands::trigger_typing,
            commands::request_members,
            commands::gateway_send_raw,
            commands::kick_member,
            commands::ban_member,
            commands::join_voice,
            commands::voice_loopback,
            // голосовой DSP: шумодав + AGC
            commands::get_voice_dsp,
            commands::set_voice_dsp,
            commands::get_voice_dsp_status,
            commands::start_stream,
            commands::stop_stream,
            commands::pause_stream,
            commands::parse_snowflake,
            // пользовательская сессия: друзья и личные беседы (v0.4.0)
            commands::create_dm,
            commands::list_relationships,
            // авторизация пользователя (v0.3.0)
            commands::auth_login_email,
            commands::auth_submit_mfa,
            commands::auth_start_qr,
            commands::auth_cancel_qr,
            // универсальный REST-прокси + управление сервером
            commands::api_request,
            commands::frontend_log,
            // v0.6.0: подписки на гильдию, обновления, файлы, логи
            commands::subscribe_guild,
            commands::subscribe_channel,
            commands::check_update,
            commands::can_auto_update,
            commands::upload_files,
            commands::get_log_path,
            // плагины
            commands::plugin_list,
            commands::plugin_save,
            commands::plugin_delete,
            commands::plugin_set_enabled,
            commands::plugin_set_meta,
            commands::plugin_read,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RustyCord");
}
