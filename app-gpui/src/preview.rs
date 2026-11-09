//! Локальное превью «Показать экран» / «Камера» для GPUI-фронта:
//! standalone wry-окно с getDisplayMedia/getUserMedia — тот же движок
//! и тот же UX, что в Tauri-версии (выбор источника в системном
//! диалоге браузера, 30 fps, нативная полоса «Остановить»).
//!
//! Архитектура — как у captcha.rs: winit/wry живут в отдельном потоке,
//! события страницы приходят перехватом навигации на
//! `https://rc-preview.local/cb?s=…` (любая такая навигация = останов),
//! окно закрывается через EventLoopProxy. Колбэк `on_stop` вызывается
//! ровно один раз: кнопка «Остановить» в окне, нативный «Стоп»
//! браузера (track ended), закрытие окна или отказ от выбора источника.
//!
//! Сигналинг (op 18 Go Live / op 4 self_video) живёт в actions.rs —
//! превью лишь показывает картинку и сообщает, когда показ закончился.
//!
//! macOS: WKWebView требует главный поток приложения, занятый GPUI, —
//! окно недоступно, Go Live/камера работают без превью (см. CHANGELOG).

// В headless-сборке (без wry) заглушки не используются actions.rs —
// приглушаем dead_code, как в captcha.rs.
#![cfg_attr(
    not(feature = "captcha-webview"),
    allow(unused_imports, unused_variables, dead_code)
)]

/* ---------------------- Сборка без webview ---------------------- */

#[cfg(not(feature = "captcha-webview"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewKind {
    Screen,
    Camera,
}

#[cfg(not(feature = "captcha-webview"))]
pub fn open(kind: PreviewKind, on_stop: Box<dyn FnOnce() + Send>) {
    // Сюда не должны попадать: actions.rs открывает превью только под
    // фичей. Если всё же вызвали — не молчим и не роняем стрим.
    tracing::warn!("preview: окно превью недоступно в этой сборке ({kind:?})");
    drop(on_stop);
}

/* ------------------------- Реализация (wry) ------------------------- */

#[cfg(feature = "captcha-webview")]
mod imp {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum PreviewKind {
        Screen,
        Camera,
    }

    /// Команды в поток окна (через winit EventLoopProxy).
    enum WinMsg {
        Close,
    }

    /// Страница превью: <video> + полоса «Остановить». Любое завершение
    /// (кнопка, нативный стоп, отмена выбора источника) уходит навигацией
    /// на rc-preview.local — её перехватывает navigation_handler.
    fn page_html(kind: PreviewKind) -> String {
        let (title, js_kind) = match kind {
            PreviewKind::Screen => ("Показ экрана", "screen"),
            PreviewKind::Camera => ("Камера", "camera"),
        };
        format!(
            r#"<!doctype html>
<html><head><meta charset="utf-8"><title>RustyCord — {title}</title>
<style>
 html,body{{margin:0;height:100%;background:#1e1f22;color:#dbdee1;
  font-family:system-ui,sans-serif;display:flex;flex-direction:column}}
 video{{flex:1;min-height:0;width:100%;object-fit:contain;background:#000}}
 .bar{{display:flex;align-items:center;gap:12px;padding:8px 12px;background:#2b2d31}}
 button{{background:#5865f2;color:#fff;border:0;border-radius:4px;
  padding:6px 14px;font-size:14px;cursor:pointer}}
 #status{{font-size:12px;color:#949ba4}}
</style></head>
<body>
<video id="v" autoplay muted playsinline></video>
<div class="bar"><button id="stop">Остановить</button><span id="status"></span></div>
<script>
const kind = "{js_kind}";
const status = s => document.getElementById('status').textContent = s;
function done(){{ location.href = 'https://rc-preview.local/cb?s=ended'; }}
document.getElementById('stop').onclick = done;
const media = kind === 'camera'
  ? navigator.mediaDevices.getUserMedia({{video:{{width:1280,height:720}},audio:false}})
  : navigator.mediaDevices.getDisplayMedia({{video:{{frameRate:30}},audio:false}});
media.then(s => {{
  const v = document.getElementById('v');
  v.srcObject = s;
  status('В эфире');
  s.getVideoTracks()[0].addEventListener('ended', done);
}}).catch(e => {{
  // Отмена выбора источника/ошибка — превью не состоялось: останавливаем.
  status('Захват не начат: ' + (e && e.message ? e.message : e));
  setTimeout(done, 900);
}});
</script>
</body></html>"#
        )
    }

    struct PreviewApp {
        kind: PreviewKind,
        /// Колбэк останова; take() гарантирует один вызов.
        on_stop: Option<Box<dyn FnOnce() + Send>>,
        window: Option<winit::window::Window>,
        webview: Option<wry::WebView>,
        /// Прокси создаётся у EventLoop до run_app (у ActiveEventLoop его нет).
        proxy: winit::event_loop::EventLoopProxy<WinMsg>,
    }

    impl PreviewApp {
        fn stop(&mut self) {
            if let Some(cb) = self.on_stop.take() {
                cb();
            }
        }
    }

    impl winit::application::ApplicationHandler<WinMsg> for PreviewApp {
        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if self.window.is_some() {
                return; // повторный resumed (мобильные) игнорируем
            }
            let (w, h) = match self.kind {
                PreviewKind::Screen => (680.0, 440.0),
                PreviewKind::Camera => (500.0, 400.0),
            };
            let attrs = winit::window::Window::default_attributes()
                .with_title(match self.kind {
                    PreviewKind::Screen => "RustyCord — показ экрана",
                    PreviewKind::Camera => "RustyCord — камера",
                })
                .with_inner_size(winit::dpi::LogicalSize::new(w, h))
                .with_min_inner_size(winit::dpi::LogicalSize::new(320.0, 240.0));
            let window = match event_loop.create_window(attrs) {
                Ok(win) => win,
                Err(e) => {
                    tracing::warn!("preview: окно не создано: {e}");
                    self.stop();
                    event_loop.exit();
                    return;
                }
            };

            let proxy = self.proxy.clone();
            let builder = wry::WebViewBuilder::new()
                .with_html(page_html(self.kind))
                // <video autoplay muted>: иначе политики автоплея гасят картинку.
                .with_autoplay(true)
                // getUserMedia (камера) без явного разрешения на Windows
                // молча реджектится; DisplayCapture на webkit2gtk тоже
                // проходит через permission-запрос.
                .with_permission_handler(|_kind| wry::PermissionResponse::Allow)
                .with_navigation_handler(move |uri: String| {
                    // Единственный канал связи со страницей: любая навигация
                    // на rc-preview.local = «показ окончен» (кнопка/стоп/отмена).
                    if uri.starts_with("https://rc-preview.local/cb") {
                        let _ = proxy.send_event(WinMsg::Close);
                        return false;
                    }
                    true
                });

            match builder.build(&window) {
                Ok(webview) => {
                    self.webview = Some(webview);
                    self.window = Some(window);
                }
                Err(e) => {
                    // Нет системного вебдвижка (webkit2gtk/WebView2) —
                    // показ без превью не оставляем.
                    tracing::warn!("preview: webview не создан: {e}");
                    self.stop();
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
                self.stop();
                event_loop.exit();
            }
        }

        fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: WinMsg) {
            match event {
                WinMsg::Close => {
                    self.stop();
                    event_loop.exit();
                }
            }
        }
    }

    /// Открыть окно превью. Блокирующий `on_stop` вызывается один раз при
    /// остановке показа — из него шлётся op 19 (экран) / op 4 (камера);
    /// задачи спавнятся на общем tokio-рантайме (см. actions.rs).
    pub fn open(kind: PreviewKind, on_stop: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            let event_loop = match winit::event_loop::EventLoop::<WinMsg>::with_user_event().build()
            {
                Ok(el) => el,
                Err(e) => {
                    tracing::warn!("preview: event loop: {e}");
                    on_stop();
                    return;
                }
            };
            let proxy = event_loop.create_proxy();
            let mut app = PreviewApp {
                kind,
                on_stop: Some(on_stop),
                window: None,
                webview: None,
                proxy,
            };
            let _ = event_loop.run_app(&mut app);
        });
    }
}

#[cfg(feature = "captcha-webview")]
pub use imp::{open, PreviewKind};
