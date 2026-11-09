//! RustyCord GPUI — нативный фронтенд на GPUI (Zed) поверх общего
//! протокольного ядра rustycord-core. Сборка без Tauri: бинарник
//! занимает мегабайты, а не сотни, и стартует мгновенно.

mod actions;
mod bridge;
mod captcha;
mod chat;
mod http;
mod login;
mod qr;
mod state;
mod text_input;
mod theme;

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
    // Логи: stdout (+ файл позже, когда появится каталог данных).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustycord=info".into()),
        )
        .with_ansi(false)
        .try_init();
    tracing::info!(
        "RustyCord GPUI {} (протокол {})",
        env!("CARGO_PKG_VERSION"),
        rustycord_core::VERSION
    );

    // Тёплый старт tokio-рантайма (gateway/voice/REST задачи ядра).
    bridge::runtime();

    Application::new().run(|cx: &mut App| {
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
                |_, cx| cx.new(|cx| AppModel::new(core.clone(), tx.clone(), cx)),
            )
            .expect("окно");

        // Pump: качает канал событий и раздаёт их в модель на главном потоке.
        let model = window.update(cx, |_, _, cx| cx.entity()).expect("модель");
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            while let Some((event, payload)) = rx.recv().await {
                let event = event.clone();
                let payload = payload.clone();
                let _ = cx.update(|cx: &mut App| {
                    model.update(cx, |m: &mut AppModel, cx: &mut Context<AppModel>| {
                        m.handle_event(&event, payload, cx)
                    });
                });
            }
        })
        .detach();

        cx.activate(true);
    });
}
