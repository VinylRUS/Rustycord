//! Капча hCaptcha/Turnstile для GPUI-фронта: standalone wry-окно на
//! discord.com (без Tauri). Логика и JS — порт боевой Tauri-версии
//! (src-tauri/src/captcha_webview.rs, v0.8.x):
//!  * скрытое окно грузит https://discord.com/login;
//!  * INIT_SCRIPT (до скриптов страницы) даёт `__rcAuth.request/challenge`;
//!  * обратная связь — перехват навигации /__rustycord_cb;
//!  * на 400+captcha окно показывается, решение уходит в повторный POST
//!    и заголовками X-Captcha-*, и полями тела (≤ 3 попыток).
//!
//! Потоковая модель: winit/wry живут в отдельном потоке (EventLoop),
//! команды (eval/reveal/close) приходят через EventLoopProxy, события
//! навигации — через unbounded-канал в асинхронный драйвер на tokio.
//!
//! macOS: WKWebView требует главный поток приложения, который занят
//! GPUI — окно капчи недоступно (QR/токен-вход работают); включим позже.

#![cfg_attr(
    not(feature = "captcha-webview"),
    allow(unused_imports, unused_variables, dead_code)
)]

use std::time::Duration;

use rustycord_core::auth::captcha::{
    BrowserPostOutcome, CaptchaChallenge, CaptchaSolver, CAPTCHA_MAX_ATTEMPTS,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

/* ---------------- Заглушка (сборка без окна капчи) ---------------- */

#[cfg(not(feature = "captcha-webview"))]
pub struct WryCaptcha;

#[cfg(not(feature = "captcha-webview"))]
#[async_trait::async_trait]
impl CaptchaSolver for WryCaptcha {
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

#[cfg(feature = "captcha-webview")]
mod solver {

    /* ------------------------- Инжектируемый JS ------------------------- */
    /* Порт INIT_SCRIPT из Tauri-версии (v0.8.3): менять можно только вместе
     * с ней — виджет hCaptcha Enterprise требует setData, а не render-опцию. */

    const INIT_SCRIPT: &str = include_str!("captcha_init.js");
    fn titles(context: &str) -> (&'static str, &'static str) {
        match context {
        "qr" => (
            "Подтверждение входа по QR-коду",
            "Discord просит подтвердить, что ты не робот, чтобы завершить вход по QR. Реши капчу — вход продолжится автоматически.",
        ),
        "login" => (
            "Подтверждение входа",
            "Discord просит подтвердить, что ты не робот. Реши капчу — вход продолжится автоматически.",
        ),
        "mfa" => (
            "Подтверждение кода 2FA",
            "Discord просит подтвердить, что ты не робот, прежде чем принять код двухфакторной аутентификации.",
        ),
        _ => (
            "Подтверждение безопасности",
            "Discord просит подтвердить, что ты не робот.",
        ),
    }
    }

    fn challenge_js(challenge: &CaptchaChallenge, context: &str) -> String {
        let (title, subtitle) = titles(context);
        let cfg = json!({
            "sitekey": challenge.sitekey,
            "rqdata": challenge.rqdata,
            "invisible": challenge.invisible,
            "title": title,
            "subtitle": subtitle,
        });
        format!("window.__rcAuth && window.__rcAuth.challenge({cfg})")
    }

    fn request_js(
        urls: &[&str],
        body: &Value,
        context_properties: Option<&str>,
        build: u64,
        captcha_key: Option<&str>,
        session_id: Option<&str>,
        rqtoken: Option<&str>,
    ) -> String {
        let cfg = json!({
            "urls": urls,
            "body": body,
            "build": build,
            "locale": "ru-RU",
            "launchId": rustycord_core::superprops::launch_id(),
            "launchSignature": rustycord_core::superprops::launch_signature(),
            "contextProperties": context_properties,
            "captchaKey": captcha_key,
            "sessionId": session_id,
            "rqtoken": rqtoken,
            "veil": if captcha_key.is_some() { "Проверяем решение…" } else { "Завершаем вход…" },
        });
        format!("window.__rcAuth && window.__rcAuth.request({cfg})")
    }

    /* ----------------------------- Реализация ----------------------------- */

    /// Команды в поток окна (через winit EventLoopProxy).
    enum WinMsg {
        Eval(String),
        Reveal,
        Close,
    }

    pub struct WryCaptcha;

    #[async_trait::async_trait]
    impl CaptchaSolver for WryCaptcha {
        async fn browser_post(
            &self,
            context: &str,
            urls: &[&str],
            body: &Value,
            context_properties: Option<&str>,
            build: u64,
        ) -> anyhow::Result<BrowserPostOutcome> {
            browser_post_impl(context, urls, body, context_properties, build).await
        }
    }

    async fn browser_post_impl(
        context: &str,
        urls: &[&str],
        body: &Value,
        context_properties: Option<&str>,
        build: u64,
    ) -> anyhow::Result<BrowserPostOutcome> {
        // Запустить поток окна; (proxy, rx навигации) вернутся по каналу.
        let (nav_tx, mut nav_rx) = mpsc::unbounded_channel::<(String, String)>();
        let (ready_tx, ready_rx) =
            std::sync::mpsc::channel::<anyhow::Result<winit::event_loop::EventLoopProxy<WinMsg>>>();

        let window_thread = std::thread::Builder::new()
            .name("rustycord-captcha".into())
            .spawn(move || run_window(nav_tx, ready_tx))
            .map_err(|e| anyhow::anyhow!("поток окна капчи: {e}"))?;

        let proxy = ready_rx
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| anyhow::anyhow!("окно подтверждения не ответило"))??;

        let outcome = drive(
            &proxy,
            &mut nav_rx,
            context,
            urls,
            body,
            context_properties,
            build,
        )
        .await;

        let _ = proxy.send_event(WinMsg::Close);
        // Поток завершает цикл сам; не ждём дольше секунды.
        let _ = window_thread.join();
        outcome
    }

    /// Драйвер цикла «POST → капча → ретрай» (порт Tauri-версии).
    async fn drive(
        proxy: &winit::event_loop::EventLoopProxy<WinMsg>,
        rx: &mut mpsc::UnboundedReceiver<(String, String)>,
        context: &str,
        urls: &[&str],
        body: &Value,
        context_properties: Option<&str>,
        build: u64,
    ) -> anyhow::Result<BrowserPostOutcome> {
        let flow = tokio::time::sleep(Duration::from_secs(300));
        tokio::pin!(flow);

        let mut challenge: Option<CaptchaChallenge> = None;
        let mut solution_pending = false;
        let mut rejections: u8 = 0;

        loop {
            tokio::select! {
                _ = &mut flow => anyhow::bail!("время на подтверждение вышло (5 минут)"),
                nav = rx.recv() => match nav {
                    Some((kind, data)) => match kind.as_str() {
                        "ready" => {
                            solution_pending = false;
                            let js = request_js(urls, body, context_properties, build, None, None, None);
                            proxy
                                .send_event(WinMsg::Eval(js))
                                .map_err(|e| anyhow::anyhow!("eval: {e}"))?;
                        }
                        "http" => {
                            let v: Value = serde_json::from_str(&data)
                                .map_err(|e| anyhow::anyhow!("странный ответ страницы: {e}"))?;
                            let status = v["status"].as_u64().unwrap_or(0);
                            let text = v["body"].as_str().unwrap_or_default().to_string();
                            if (200..300).contains(&status) {
                                return Ok(BrowserPostOutcome::Ok(text));
                            }
                            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                            if status == 400 {
                                if solution_pending {
                                    rejections += 1;
                                }
                                solution_pending = false;
                                if let Some(next) = CaptchaChallenge::from_body(&parsed) {
                                    if rejections >= CAPTCHA_MAX_ATTEMPTS {
                                        anyhow::bail!(
                                            "капча: Discord отклонил решение {CAPTCHA_MAX_ATTEMPTS} раз подряд"
                                        );
                                    }
                                    let raw: String = text.chars().take(2000).collect();
                                    tracing::warn!(
                                        "captcha-wry: HTTP 400 + капча (отказов {rejections}), ответ: {raw}"
                                    );
                                    let js = challenge_js(&next, context);
                                    challenge = Some(next);
                                    proxy
                                        .send_event(WinMsg::Eval(js))
                                        .map_err(|e| anyhow::anyhow!("eval: {e}"))?;
                                    proxy
                                        .send_event(WinMsg::Reveal)
                                        .map_err(|e| anyhow::anyhow!("reveal: {e}"))?;
                                    continue;
                                }
                            }
                            let snippet: String = text.chars().take(300).collect();
                            tracing::warn!("captcha-wry: HTTP {status} — {snippet}");
                            return Ok(BrowserPostOutcome::Rejected(text));
                        }
                        "captcha" => {
                            let prefix: String = data.chars().take(10).collect();
                            tracing::warn!(
                                "captcha-wry: решение получено (len={}, «{prefix}…») — повторяем",
                                data.len()
                            );
                            let Some(ch) = challenge.as_ref() else {
                                solution_pending = false;
                                let js =
                                    request_js(urls, body, context_properties, build, None, None, None);
                                proxy
                                    .send_event(WinMsg::Eval(js))
                                    .map_err(|e| anyhow::anyhow!("eval: {e}"))?;
                                continue;
                            };
                            solution_pending = true;
                            let js = request_js(
                                urls,
                                body,
                                context_properties,
                                build,
                                Some(&data),
                                ch.session_id.as_deref(),
                                ch.rqtoken.as_deref(),
                            );
                            proxy
                                .send_event(WinMsg::Eval(js))
                                .map_err(|e| anyhow::anyhow!("eval: {e}"))?;
                        }
                        "hcaptcha-err" => {
                            tracing::warn!("captcha-wry: ошибка виджета hCaptcha — {data}");
                        }
                        "err" => anyhow::bail!("страница подтверждения: {data}"),
                        "cancel" => anyhow::bail!("отменено пользователем"),
                        "closed" => anyhow::bail!("окно подтверждения закрыто"),
                        _ => {}
                    },
                    None => anyhow::bail!("окно подтверждения закрыто"),
                },
            }
        }
    }

    /* ------------------------- Поток окна (winit+wry) ------------------------- */

    #[cfg(target_os = "linux")]
    type WinitEventLoop = winit::event_loop::EventLoop<WinMsg>;
    #[cfg(not(target_os = "linux"))]
    type WinitEventLoop = winit::event_loop::EventLoop<WinMsg>;

    struct CaptchaApp {
        nav_tx: mpsc::UnboundedSender<(String, String)>,
        window: Option<winit::window::Window>,
        webview: Option<wry::WebView>,
        ready_tx: Option<
            std::sync::mpsc::Sender<anyhow::Result<winit::event_loop::EventLoopProxy<WinMsg>>>,
        >,
    }

    impl winit::application::ApplicationHandler<WinMsg> for CaptchaApp {
        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if self.window.is_some() {
                return; // уже создано (мобильный resumed может дублироваться)
            }
            let attrs = winit::window::Window::default_attributes()
                .with_title("RustyCord — подтверждение Discord")
                .with_inner_size(winit::dpi::LogicalSize::new(500.0, 560.0))
                .with_min_inner_size(winit::dpi::LogicalSize::new(440.0, 460.0))
                .with_visible(false)
                .with_resizable(true);
            let window = match event_loop.create_window(attrs) {
                Ok(w) => w,
                Err(e) => {
                    if let Some(tx) = self.ready_tx.take() {
                        let _ = tx.send(Err(anyhow::anyhow!("окно: {e}")));
                    }
                    event_loop.exit();
                    return;
                }
            };

            let nav_tx = self.nav_tx.clone();
            let builder = wry::WebViewBuilder::new()
                .with_url("https://discord.com/login")
                .unwrap_or_else(|_| wry::WebViewBuilder::new())
                .with_initialization_script(INIT_SCRIPT)
                .with_navigation_handler(move |uri: String| {
                    // Единственный канал связи: блокируем «запрещённую»
                    // навигацию и читаем её параметры (/__rustycord_cb?s=&d=).
                    if let Some((path, query)) = uri.split_once('?') {
                        if path.ends_with("/__rustycord_cb") {
                            let mut kind = String::new();
                            let mut data = String::new();
                            for pair in query.split('&') {
                                let (k, v) = pair.split_once('=').unwrap_or(("", ""));
                                match k {
                                    "s" => kind = percent_decode(v),
                                    "d" => data = percent_decode(v),
                                    _ => {}
                                }
                            }
                            let _ = nav_tx.send((kind, data));
                            return false;
                        }
                    }
                    true
                });

            #[cfg(target_os = "linux")]
            let build_result = builder.build(&window);
            #[cfg(not(target_os = "linux"))]
            let build_result = builder.build(&window);

            match build_result {
                Ok(webview) => {
                    self.webview = Some(webview);
                    self.window = Some(window);
                    if let Some(tx) = self.ready_tx.take() {
                        let _ = tx.send(Ok(event_loop.create_proxy()));
                    }
                }
                Err(e) => {
                    if let Some(tx) = self.ready_tx.take() {
                        let _ = tx.send(Err(anyhow::anyhow!("webview: {e}")));
                    }
                    event_loop.exit();
                }
            }
        }

        fn window_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            _window_id: winit::window::WindowId,
            event: winit::event::WindowEvent,
        ) {
            if let winit::event::WindowEvent::CloseRequested = event {
                let _ = self.nav_tx.send(("closed".into(), String::new()));
                event_loop.exit();
            }
        }

        fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: WinMsg) {
            match event {
                WinMsg::Eval(js) => {
                    if let Some(wv) = &self.webview {
                        if let Err(e) = wv.evaluate_script(&js) {
                            tracing::warn!("captcha-wry: eval: {e}");
                        }
                    }
                }
                WinMsg::Reveal => {
                    if let Some(w) = &self.window {
                        w.set_visible(true);
                        let _ = w.focus_window();
                    }
                }
                WinMsg::Close => {
                    event_loop.exit();
                }
            }
        }
    }

    fn run_window(
        nav_tx: mpsc::UnboundedSender<(String, String)>,
        ready_tx: std::sync::mpsc::Sender<
            anyhow::Result<winit::event_loop::EventLoopProxy<WinMsg>>,
        >,
    ) {
        let event_loop = match winit::event_loop::EventLoop::<WinMsg>::with_user_event().build() {
            Ok(el) => el,
            Err(e) => {
                let _ = ready_tx.send(Err(anyhow::anyhow!("event loop: {e}")));
                return;
            }
        };
        let mut app = CaptchaApp {
            nav_tx,
            window: None,
            webview: None,
            ready_tx: Some(ready_tx),
        };
        let _ = event_loop.run_app(&mut app);
    }

    /// Минимальный percent-decode (компоненты URI): %XX → байт, «+» → пробел.
    fn percent_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' if i + 2 < bytes.len() => {
                    if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                        out.push(b);
                        i += 3;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
} // mod solver

#[cfg(feature = "captcha-webview")]
pub use solver::WryCaptcha;
