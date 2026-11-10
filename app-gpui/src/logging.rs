//! Журналирование и перехват паник: все логи и паники дублируются в файл
//! `<config>/logs/rustycord.log` — даже в GUI-сборке Windows, где консоли
//! нет и stderr теряется (v0.12.1 «вылетает без ошибок»).
//!
//! Что пишется: события уровня info (обычный режим) или debug (dev-режим),
//! ошибки протокола и ПАНИКИ с местом в коде. Ротация: при старте файл
//! больше 5 МБ переезжает в `rustycord.old.log`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;

/// Каталог логов: рядом с config.json.
pub fn logs_dir() -> PathBuf {
    crate::config::config_dir().join("logs")
}

/// Текущий лог-файл (открывается один раз в [`init`]).
static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

/// Максимальный размер лога до ротации.
const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;

/// Инициализация: открыть лог-файл, установить panic-хук, подписать
/// tracing-сабскрайбер (консоль + файл). Вызывается один раз на старте,
/// до первого лога.
pub fn init(dev_mode: bool) {
    let dir = logs_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        // Нет папки логов — работаем без файла, но об этом надо сказать.
        eprintln!("logging: не создать {}: {e}", dir.display());
    }
    let path = dir.join("rustycord.log");
    rotate_if_huge(&path);
    let file = OpenOptions::new().create(true).append(true).open(&path);
    match file {
        Ok(f) => {
            let _ = LOG_FILE.set(Mutex::new(Some(f)));
            // Первый заход: отметка запуска, чтобы искать начало сессии.
            append_line(&format!(
                "\n=== RustyCord GPUI {} запущен {} ===",
                env!("CARGO_PKG_VERSION"),
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            ));
        }
        Err(e) => {
            eprintln!("logging: не открыть {}: {e}", path.display());
            let _ = LOG_FILE.set(Mutex::new(None));
        }
    }

    install_panic_hook();

    // Уровень логов: dev-режим поднимает до debug. Сабскрайбер пишет и в
    // файл, и в консоль (Windows — WriteConsoleW, см. devconsole в main).
    let default_filter = if dev_mode {
        "rustycord=debug,rustycord_core=debug"
    } else {
        "rustycord=info"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(ComboWriter)
        .try_init();
}

/// Ротация: файл > 5 МБ → rustycord.old.log (предыдущий old удаляется;
/// на Windows rename поверх существующего файла не работает).
fn rotate_if_huge(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() <= ROTATE_AT_BYTES {
        return;
    }
    let old = path.with_extension("old.log");
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(path, &old) {
        eprintln!("logging: ротация не удалась: {e}");
    }
}

/// Panic-хук: паника обязана попасть в лог-файл — в GUI-сборке это
/// единственный след вылета. После записи зовём предыдущий хук
/// (консоль/stderr в dev-режиме).
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        append_line(&format!(
            "ПАНИКА: {info}\n  поток: {:?}\n  backtrace:\n{}",
            std::thread::current().name().unwrap_or("<без имени>"),
            std::backtrace::Backtrace::force_capture()
        ));
        prev(info);
    }));
}

/// Дописать строку в лог-файл (используется и вне tracing — паники).
fn append_line(line: &str) {
    let Some(cell) = LOG_FILE.get() else {
        return;
    };
    if let Ok(mut guard) = cell.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

/// Приёмник tracing: каждая запись уходит в файл и (на Windows) в консоль.
struct ComboWriter;

impl<'a> MakeWriter<'a> for ComboWriter {
    type Writer = ComboSink;

    fn make_writer(&'a self) -> ComboSink {
        ComboSink
    }
}

/// Пишем и в файл, и в консоль. Ошибки глотаем: логирование не имеет
/// права уронить приложение (особенно из panic-хука).
struct ComboSink;

impl std::io::Write for ComboSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(cell) = LOG_FILE.get() {
            if let Ok(mut guard) = cell.lock() {
                if let Some(f) = guard.as_mut() {
                    let _ = f.write_all(buf);
                }
            }
        }
        // Консоль: на Windows — WriteConsoleW (без консоли тихо пропадает),
        // на Unix — stdout как раньше.
        #[cfg(target_os = "windows")]
        crate::devconsole::write_console(buf);
        #[cfg(not(target_os = "windows"))]
        let _ = std::io::stdout().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(cell) = LOG_FILE.get() {
            if let Ok(mut guard) = cell.lock() {
                if let Some(f) = guard.as_mut() {
                    let _ = f.flush();
                }
            }
        }
        Ok(())
    }
}

/// Открыть папку логов в системном файловом менеджере (кнопка в
/// «Настройки → Диагностика»). Папку создаём заранее — иначе откроется
/// родитель или ничего.
pub fn open_logs_dir() {
    let dir = logs_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("logging: не создать {}: {e}", dir.display());
        return;
    }
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    if let Err(e) = std::process::Command::new(cmd).arg(&dir).spawn() {
        tracing::warn!("logging: не открыть {}: {e} (путь: {})", cmd, dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_dir_lives_next_to_config() {
        let dir = logs_dir();
        assert!(dir.ends_with("logs"), "странный путь: {}", dir.display());
    }
}
