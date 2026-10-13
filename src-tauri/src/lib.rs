mod auth;
mod commands;
mod gateway;
mod rest;
mod snowflake;
mod state;
mod superprops;
mod voice;
mod wsutil;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Structured logs for the reverse-engineering playground.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustycord=info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(Arc::new(AppState::new(handle)));
            tracing::info!("RustyCord core is up (Tauri {})", tauri::VERSION);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::disconnect,
            commands::get_state,
            commands::get_dm_channels,
            commands::get_messages,
            commands::send_message,
            commands::edit_message,
            commands::delete_message,
            commands::add_reaction,
            commands::trigger_typing,
            commands::request_members,
            commands::gateway_send_raw,
            commands::kick_member,
            commands::ban_member,
            commands::join_voice,
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
