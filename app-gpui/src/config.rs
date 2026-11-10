//! Локальная конфигурация GPUI-клиента: токен сессии и dev-режим.
//!
//! Файл `config.json` в каталоге данных платформы:
//!   Windows: %APPDATA%\rustycord\config.json
//!   macOS:   ~/Library/Application Support/rustycord/config.json
//!   Linux:   $XDG_CONFIG_HOME/rustycord/config.json (~/.config/…)
//!
//! Токен хранится как в любом локальном клиенте (тот же профиль риска,
//! что Vencord/Vesktop); на Unix права 0600. Запись атомарная
//! (tmp + rename), чтобы убийство процесса не оставляло битый файл.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Токен последней успешной сессии (автологин при запуске).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Dev-режим: консоль с логами (Windows) + уровень debug.
    #[serde(default)]
    pub dev_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: None,
            dev_mode: false,
        }
    }
}

/// Каталог данных приложения (без платформенных крейтов: env-переменные
/// дают те же пути, что `dirs`/Tauri на всех трёх ОС).
pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        // %APPDATA% — Roaming; создание берёт на себя save().
        let base = std::env::var("APPDATA")
            .or_else(|_| std::env::var("USERPROFILE").map(|p| p + "\\.config"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("rustycord")
    } else if cfg!(target_os = "macos") {
        let base = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(base)
            .join("Library")
            .join("Application Support")
            .join("rustycord")
    } else {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                home + "/.config"
            });
        PathBuf::from(base).join("rustycord")
    }
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Прочитать конфиг (отсутствующий/битый файл = дефолт, не ошибка:
/// клиент обязан стартовать даже с повреждённым конфигом).
pub fn load() -> Config {
    let Ok(raw) = fs::read_to_string(config_path()) else {
        return Config::default();
    };
    match serde_json::from_str(&raw) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(
                "config: {} не читается ({e}) — используем дефолт",
                config_path().display()
            );
            Config::default()
        }
    }
}

/// Записать конфиг атомарно (tmp + rename) с правами 0600 на Unix.
pub fn save(cfg: &Config) {
    let path = config_path();
    let dir = config_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("config: не создать {}: {e}", dir.display());
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(cfg) else {
        return;
    };
    let tmp = dir.join(".config.json.tmp");
    if let Err(e) = fs::write(&tmp, json.as_bytes()) {
        tracing::warn!("config: не записать {}: {e}", tmp.display());
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        tracing::warn!("config: не переименовать в {}: {e}", path.display());
    }
}

impl Config {
    pub fn set_token(&mut self, token: &str) {
        self.token = Some(token.trim().to_string());
        save(self);
    }

    pub fn clear_token(&mut self) {
        if self.token.take().is_some() {
            save(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let cfg = Config {
            token: Some("abc".into()),
            dev_mode: true,
        };
        let raw = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.token.as_deref(), Some("abc"));
        assert!(back.dev_mode);
    }

    #[test]
    fn default_is_login_off() {
        let cfg = Config::default();
        assert!(cfg.token.is_none());
        assert!(!cfg.dev_mode);
    }

    #[test]
    fn corrupt_file_is_default() {
        // serde_json::from_str::<Config>("{oops") → Err → дефолт на месте вызова.
        assert!(serde_json::from_str::<Config>("{oops").is_err());
    }
}
