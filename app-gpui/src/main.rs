//! RustyCord GPUI — нативный фронтенд на GPUI (Zed) поверх общего
//! протокольного ядра rustycord-core. Сборка без Tauri: бинарник
//! занимает мегабайты, а не сотни, и стартует мгновенно.
//!
//! v0.12.1: токен сессии переживает перезапуск (автологин), клик по ✕
//! прячет окно в трей (полный выход — «Выход» в меню значка), консоль
//! опциональна через dev-режим в настройках.

// Windows: без консоли при двойном клике. Консоль появляется только
// в dev-режиме (AllocConsole на старте) или при запуске из терминала.
// Значение именно "windows" (= подсистема GUI): rustc 1.99 валидирует
// ввод и разрешает только "console" | "windows" — «gui» даёт E0539.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod actions;
mod assets;
mod bridge;
mod captcha;
mod chat;
mod config;
#[cfg(target_os = "windows")]
mod devconsole;
mod http;
mod logging;
mod login;
mod preview;
mod qr;
mod settings;
mod state;
mod text_input;
mod theme;
mod tray;

use std::sync::Arc;

use gpui::{
    actions, px, size, App, AppContext as _, Application, Bounds, Context, IntoElement, Render,
    Window, WindowBounds, WindowOptions,
};
actions!(rustycord, [Quit]);
use rustycord_core::state::AppState;

use bridge::GpuiSink;
use state::{AppModel, Screen};

/// Заглушка капчи для сборок без окна (нет webkit2gtk/WebView2 или macOS).
#[cfg(not(feature = "captcha-webview"))]
mod captcha_stub {
    use rustycord_core::auth::captcha::{BrowserPostOutcome, CaptchaSolver};
    use serde_json::Value;

    pub struct StubCaptcha;

    #[async_trait::async_trait]
    impl CaptchaSolver for StubCaptcha {
        async fn browser_post(
            &self,
            _context: &str,
            _urls: &[&str],
            _body: &Value,
            _ctx_props: Option<&str>,
            _build: u64,
        ) -> anyhow::Result<BrowserPostOutcome> {
            anyhow::bail!(
                "Окно капчи недоступно в этой сборке (нет системного вебдвижка). \
                 Войди по токену или QR-коду без капчи."
            )
        }
    }
}

/// URL-энкодинг для реакций (как в Tauri-версии).
pub use rustycord_core::rest::client::urlencode;

impl Render for AppModel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.screen {
            Screen::Login(_) => login::render(self, window, cx).into_any_element(),
            Screen::Main(_) => chat::render(self, window, cx).into_any_element(),
        }
    }
}

fn main() {
    // 1. Конфиг (токен + dev-режим) — до всего остального.
    let cfg = config::load();

    // 2. Dev-консоль (Windows): AllocConsole до первого лога.
    #[cfg(target_os = "windows")]
    if cfg.dev_mode {
        devconsole::setup();
    }

    // 3. Логи: файл <config>/logs/rustycord.log + консоль + panic-хук.
    //    В GUI-сборке Windows консоли нет — файл остаётся единственным
    //    следом при вылете (v0.12.1 «вылетает без ошибок» — больше не так).
    logging::init(cfg.dev_mode);

    tracing::info!(
        "RustyCord GPUI {} (протокол {}), dev-режим: {}, логи: {}",
        env!("CARGO_PKG_VERSION"),
        rustycord_core::VERSION,
        cfg.dev_mode,
        logging::logs_dir().display()
    );

    // 4. Тёплый старт tokio-рантайма (gateway/voice/REST задачи ядра).
    bridge::runtime();

    let saved_token = cfg.token.clone().filter(|t| !t.is_empty());
    let boot_cfg = cfg.clone();

    Application::new().run(move |cx: &mut App| {
        // HTTP-клиент для картинок (аватарки/эмодзи через img()).
        cx.set_http_client(http::shared());

        // Глобальные клавиши текстовых полей + действия окна.
        cx.bind_keys(text_input::default_keybindings());
        cx.bind_keys([gpui::KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        // Канал событий «ядро → UI» и модель.
        let (tx, mut rx) = bridge::event_channel();
        let sink = Arc::new(GpuiSink(tx.clone()));

        #[cfg(feature = "captcha-webview")]
        let solver: Arc<dyn rustycord_core::auth::captcha::CaptchaSolver> =
            Arc::new(captcha::WryCaptcha);
        #[cfg(not(feature = "captcha-webview"))]
        let solver: Arc<dyn rustycord_core::auth::captcha::CaptchaSolver> =
            Arc::new(captcha_stub::StubCaptcha);

        let core = Arc::new(AppState::new(sink, solver));

        let window_bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("RustyCord".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    // Клик по ✕: прячем окно в трей, процесс живёт
                    // (выход — «Выход» в меню значка в трее).
                    window.on_window_should_close(&*cx, |window, cx| {
                        tray::hide_window(window, cx);
                        false
                    });
                    cx.new(|cx| AppModel::new(core.clone(), tx.clone(), boot_cfg.clone(), cx))
                },
            )
            .expect("окно");

        // Pump: качает канал событий и раздаёт их в модель на главном потоке.
        // Трей-события применяются к окну напрямую (модель их не касается).
        let model = window.update(cx, |_, _, cx| cx.entity()).expect("модель");
        let handle = gpui::AnyWindowHandle::from(window);
        tray::init(handle, tx.clone());

        let pump_model = model.clone();
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            while let Some((event, payload)) = rx.recv().await {
                if event == "ui://tray-open" {
                    let _ = cx.update(|cx: &mut App| tray::show_from_tray(cx));
                    continue;
                }
                if event == "ui://tray-quit" {
                    let _ = cx.update(|cx: &mut App| cx.quit());
                    continue;
                }
                let event = event.clone();
                let payload = payload.clone();
                let _ = cx.update(|cx: &mut App| {
                    pump_model.update(cx, |m: &mut AppModel, cx: &mut Context<AppModel>| {
                        m.handle_event(&event, payload, cx)
                    });
                });
            }
        })
        .detach();

        // Автологин сохранённым токеном: одно подключение на запуск —
        // никаких «5 авторизаций в минуту».
        if let Some(token) = saved_token {
            model.update(cx, |m: &mut AppModel, cx| {
                crate::actions::connect_token(m, token, cx);
            });
        }

        cx.activate(true);
    });
}
