mod commands;
mod gateway;
mod rest;
mod snowflake;
mod state;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running RustyCord");
}
