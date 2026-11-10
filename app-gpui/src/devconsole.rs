//! Dev-консоль (Windows): AllocConsole при запуске из проводника,
//! цепляние к родительской при запуске из терминала, UTF-8/UTF-16 вывод
//! через WriteConsoleW — работает и в GUI-сборке, где stdout не подключён
//! (без консоли записи тихо пропадают).
//!
//! Сам вывод логов в консоль делает logging::ComboWriter через
//! [`write_console`] — отдельный MakeWriter больше не нужен.

use windows::Win32::System::Console::{
    AllocConsole, AttachConsole, GetStdHandle, SetConsoleOutputCP, WriteConsoleW,
    ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
};

/// Подключить консоль: если запущены из терминала — цепляемся к
/// родительской, иначе создаём свою. Плюс UTF-8 для русского текста.
pub fn setup() {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            let _ = AllocConsole();
        }
        let _ = SetConsoleOutputCP(65001);
    }
}

/// Записать байты в консоль (UTF-8 → UTF-16). Без консоли — тихо.
pub fn write_console(buf: &[u8]) {
    unsafe {
        let Ok(handle) = GetStdHandle(STD_OUTPUT_HANDLE) else {
            return; // консоли нет — тихо
        };
        if handle.is_invalid() {
            return;
        }
        let utf16: Vec<u16> = String::from_utf8_lossy(buf).encode_utf16().collect();
        let mut written = 0u32;
        let _ = WriteConsoleW(handle, &utf16, Some(&mut written), None);
    }
}
