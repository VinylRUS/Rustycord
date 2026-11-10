//! Трей панели задач + «закрытие в трей».
//!
//! Клик по ✕ больше не убивает клиент (и авторизацию): окно прячется,
//! процесс живёт в трее рядом с часами. Полный выход — «Выход» в
//! контекстном меню значка (как просили: через скрытые значки, там
//! где стрелочка справа).
//!
//! Платформы:
//!   Windows — SW_HIDE/SW_SHOW по HWND (raw-window-handle от gpui);
//!   macOS   — cx.hide()/activate(true) (NSApp);
//!   Linux   — минимизация + _NET_ACTIVE_WINDOW/xdg-activation. На Wayland
//!             протокол не позволяет поднять окно принудительно — компо-
//!             зитор сам решает; на X11 работает как ожидается.
//!
//! Трей: tray-icon 0.26 (Windows/macOS нативно, Linux — D-Bus
//! StatusNotifierItem через ksni, без GTK). События меню/клика прихо-
//! дят в основной поток из wndproc/DBus и пересылаются в канал
//! моста (`ui://tray-*`), pump в main.rs применяет их к окну.

use std::sync::OnceLock;

use gpui::{AnyWindowHandle, App, Window};

use crate::bridge::EventTx;

/// Хэндл главного окна (прятать/показывать по нему).
static WINDOW: OnceLock<AnyWindowHandle> = OnceLock::new();

/// Создать трей-иконку и меню; повесить обработчики событий.
/// Вызывается один раз из главного потока (там, где gpui крутит
/// message loop — Windows/macOS этого требуют).
pub fn init(window: AnyWindowHandle, tx: EventTx) {
    let _ = WINDOW.set(window);

    let menu_tx = tx.clone();
    tray_icon::menu::MenuEvent::set_event_handler(Some(
        move |event: tray_icon::menu::MenuEvent| match event.id.0.as_str() {
            "open" => {
                let _ = menu_tx.send(("ui://tray-open".into(), serde_json::json!({})));
            }
            "quit" => {
                let _ = menu_tx.send(("ui://tray-quit".into(), serde_json::json!({})));
            }
            _ => {}
        },
    ));

    let click_tx = tx;
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
        // Левый клик по значку — открыть окно.
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            let _ = click_tx.send(("ui://tray-open".into(), serde_json::json!({})));
        }
    }));

    let menu = tray_icon::menu::Menu::new();
    let open = tray_icon::menu::MenuItem::with_id(
        "open",
        "Открыть RustyCord",
        true,
        None::<tray_icon::menu::accelerator::Accelerator>,
    );
    let quit = tray_icon::menu::MenuItem::with_id(
        "quit",
        "Выход",
        true,
        None::<tray_icon::menu::accelerator::Accelerator>,
    );
    let _ = menu.append(&open);
    let _ = menu.append(&tray_icon::menu::PredefinedMenuItem::separator());
    let _ = menu.append(&quit);

    let icon = crate::assets::tray_rgba()
        .and_then(|(rgba, w, h)| tray_icon::Icon::from_rgba(rgba, w, h).ok());

    let mut builder = tray_icon::TrayIconBuilder::new()
        .with_id("rustycord-tray")
        .with_tooltip("RustyCord — неофициальный клиент Discord на Rust")
        .with_menu(Box::new(menu));
    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }
    match builder.build() {
        Ok(tray) => {
            // Держим иконку до конца процесса: дроп убрал бы её из трея.
            // Утечка осознанная (один объект на всё время жизни приложения).
            std::mem::forget(tray);
            tracing::info!("tray: значок создан (закрытие окна прячет его в трей)");
        }
        Err(e) => {
            tracing::warn!("tray: не создать значок — окно будет закрываться как обычно: {e}")
        }
    }
}

/// Скрыть окно (клик по ✕): veto закрытия + платформенное скрытие.
/// Возвращает false → gpui отменяет закрытие.
pub fn hide_window(window: &mut Window, _cx: &mut App) {
    #[cfg(target_os = "windows")]
    {
        if hide_win32(window) {
            return;
        }
        // Нет HWND — хоть минимизируем, лишь бы не закрылось.
        window.minimize_window();
    }
    #[cfg(target_os = "macos")]
    {
        // NSApp hide: убирает окна приложения (трей остаётся).
        let _ = window;
        _cx.hide();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // X11: минимизация — WM спрячет окно в панель задач.
        // Wayland: протокола «спрятать» тоже нет — минимизация.
        window.minimize_window();
    }
}

/// Показать окно (клик по значку / «Открыть RustyCord»).
pub fn show_from_tray(cx: &mut App) {
    let Some(handle) = WINDOW.get() else {
        return;
    };
    let _ = handle.update(cx, |_, window, _cx| {
        #[cfg(target_os = "windows")]
        {
            if show_win32(window) {
                return;
            }
        }
        #[cfg(target_os = "macos")]
        {
            _cx.activate(true);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            _cx.activate(true);
        }
        // Поднять/восстановить окно (X11: _NET_ACTIVE_WINDOW — WM
        // разворачивает из минимизации).
        window.activate_window();
    });
}

/* --------------------------- Windows (Win32) --------------------------- */

#[cfg(target_os = "windows")]
fn hwnd_of(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return None;
    };
    match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(windows::Win32::Foundation::HWND(
            w.hwnd.get() as *mut core::ffi::c_void
        )),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn hide_win32(window: &mut Window) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    let Some(hwnd) = hwnd_of(window) else {
        return false;
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    true
}

#[cfg(target_os = "windows")]
fn show_win32(window: &mut Window) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindow, ShowWindowAsync, SW_RESTORE, SW_SHOW,
    };
    let Some(hwnd) = hwnd_of(window) else {
        return false;
    };
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        // Фокус: SetForegroundWindow требует «ввода» — gpui::activate
        // справляется лучше, дублируем простым вызовом.
        let _ = SetForegroundWindow(hwnd);
    }
    true
}
