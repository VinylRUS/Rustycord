//! Капча hCaptcha и капча-обмены — в отдельном окне на discord.com.
//!
//! Двойная причина (живые логи v0.8.0/v0.8.1):
//!  1. Sitekey Discord привязан к домену: на origin приложения
//!     (tauri://localhost) виджет рисуется с красным «Incorrect domain!»,
//!     клик даёт error-callback `network-error` — проверено тест-стендом.
//!  2. Enterprise-rqdata привязан к контексту сессии: rqdata из 400-ответа
//!     на reqwest-запрос (свой TLS/UA/куки) браузерный виджет отклоняет
//!     страницей «Access Denied» (rd0ah) — виджет мелькает и гаснет.
//!     Поэтому не только виджет, но и сам POST, породивший вызов капчи,
//!     обязан уйти из того же браузерного контекста: rqdata рождается
//!     и решается в одном окне.
//!
//! Архитектура: скрытое окно-WebView на https://discord.com/login; в
//! страницу до её скриптов инжектируется INIT_SCRIPT (исполняется в каждом
//! документе окна — фильтруется по discord.com):
//!  * `__rcAuth.request(cfg)` — POST на эндпоинты Discord (список с
//!    фолбеком по 404: v9 → v10) с X-Super-Properties, собранным из
//!    настоящего navigator, X-Context-Properties и X-Captcha-* на ретрае;
//!  * `__rcAuth.challenge(cfg)` — карточка с виджетом hCaptcha (sitekey +
//!    rqdata из вызова, порождённого ЭТИМ ЖЕ окном);
//!  * пока страница грузится и POST летит — «вуаль» поверх чужого логина.
//!
//! Окно скрыто, пока Discord не попросил капчу: если браузерный контекст
//! чист и 2xx приходит сразу, пользователь вообще не видит окна.
//!
//! Обратная связь — перехват навигации: инжектированный JS делает
//! `location.href = '/__rustycord_cb?s=<kind>&d=<data>'`, on_navigation
//! блокирует переход и отдаёт данные в Rust-канал. Виды событий:
//! ready | http | captcha | hcaptcha-err | err | cancel | closed.
//! CSP discord.com это позволяет: *.hcaptcha.com разрешён в
//! script-src/frame-src/style-src/connect-src (страница логина сама
//! показывает капчу).

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::mpsc;

use super::captcha::{CaptchaChallenge, CAPTCHA_MAX_ATTEMPTS};

/// Метка окна авторизации (одно на приложение).
const LABEL: &str = "rustycord-auth";
/// Стартовая страница: origin discord.com обязателен для sitekey капчи, а
/// куки/fingerprint настоящего браузера улучшают риск-скоринг.
const START_URL: &str = "https://discord.com/login";
/// Общий бюджет на весь визит окна: загрузка страницы + POST + решение
/// капчи. Первый `ready` внутри этого окна обязан прийти за десятки секунд.
const FLOW_TIMEOUT: Duration = Duration::from_secs(300);

/// Инжектируется в окно авторизации до скриптов страницы (на каждый заход).
const INIT_SCRIPT: &str = r#"
(function () {
  'use strict';
  // Скрипт исполняется в КАЖДОМ документе окна (до навигации это
  // about:blank, дальше — iframe hCaptcha): рабочий контекст — только
  // главное окно на discord.com.
  if (window.top !== window) return;
  var host = location.hostname;
  if (host !== 'discord.com' && host.slice(-13) !== '.discord.com') return;
  if (window.__rcAuthInstalled) return;
  window.__rcAuthInstalled = true;
  function cb(kind, data) {
    location.href = '/__rustycord_cb?s=' + encodeURIComponent(kind) + '&d=' + encodeURIComponent(data);
  }
  function ping() { cb('ready', '1'); }
  function onReady() {
    veil('Подключаемся к Discord…');
    ping();
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', onReady);
  else onReady();

  // Вуаль поверх чужой страницы логина: окно открывается скрытым и его
  // содержимое до капчи/закрытия не должно выглядеть интерактивным.
  function veil(text) {
    var v = document.getElementById('__rc_veil');
    if (!v) {
      v = document.createElement('div');
      v.id = '__rc_veil';
      v.style.cssText = 'position:fixed;inset:0;z-index:2147483646;background:#1e1f22;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:16px;font-family:"gg sans","Segoe UI",system-ui,sans-serif';
      var spin = document.createElement('div');
      spin.style.cssText = 'width:30px;height:30px;border:3px solid #5865f2;border-top-color:transparent;border-radius:50%;animation:__rc_spin .8s linear infinite';
      var logo = document.createElement('div');
      logo.textContent = 'RUSTYCORD';
      logo.style.cssText = 'font-size:12px;font-weight:700;letter-spacing:.14em;color:#f2f3f5';
      var label = document.createElement('div');
      label.id = '__rc_veil_text';
      label.style.cssText = 'font-size:14px;color:#b5bac1';
      v.appendChild(spin); v.appendChild(logo); v.appendChild(label);
      (document.body || document.documentElement).appendChild(v);
      var st = document.createElement('style');
      st.textContent = '@keyframes __rc_spin{to{transform:rotate(360deg)}}';
      (document.head || document.documentElement).appendChild(st);
    }
    var t = document.getElementById('__rc_veil_text');
    if (t && text) t.textContent = text;
    return v;
  }

  // X-Super-Properties из НАСТОЯЩЕГО navigator: браузерный контекст окна
  // консистентен сам с собой (UA страницы = UA запроса = UA в XSP).
  function xsp(build) {
    var ua = navigator.userAgent || '';
    var os = 'Unknown', osv = '';
    var mnt = ua.match(/Windows NT ([0-9.]+)/);
    if (mnt) { os = 'Windows'; osv = mnt[1] === '10.0' ? '10' : mnt[1]; }
    else if (/Mac OS X/.test(ua)) { os = 'Mac OS X'; }
    else if (/Android/.test(ua)) { os = 'Android'; }
    else if (/(iPhone|iPad)/.test(ua)) { os = 'iOS'; }
    else if (/Linux/.test(ua)) { os = 'Linux'; }
    var browser = 'Chrome';
    if (/Firefox\//.test(ua)) browser = 'Firefox';
    else if (/Edg\//.test(ua)) browser = 'Edge';
    else if (/Chrome\//.test(ua)) browser = 'Chrome';
    else browser = 'Safari';
    var bv = (ua.match(new RegExp(browser + '/([0-9.]+)')) || [])[1] || '';
    var p = {
      os: os, browser: browser, device: '',
      system_locale: (navigator.language || 'en-US'),
      browser_user_agent: ua, browser_version: bv, os_version: osv,
      referrer: '', referring_domain: '', referrer_current: '', referring_domain_current: '',
      release_channel: 'stable',
      client_build_number: build || 0,
      client_event_source: null, design_id: 0
    };
    try { return btoa(unescape(encodeURIComponent(JSON.stringify(p)))); } catch (e) { return ''; }
  }

  function headers(cfg) {
    var h = {
      'Content-Type': 'application/json',
      'X-Discord-Locale': cfg.locale || 'ru-RU',
      'X-Super-Properties': xsp(cfg.build)
    };
    if (cfg.contextProperties) h['X-Context-Properties'] = cfg.contextProperties;
    if (cfg.captchaKey) h['X-Captcha-Key'] = cfg.captchaKey;
    if (cfg.sessionId) h['X-Captcha-Session-Id'] = cfg.sessionId;
    if (cfg.rqtoken) h['X-Captcha-Rqtoken'] = cfg.rqtoken;
    return h;
  }

  function renderCard(cfg) {
    var old = document.getElementById('__rc_overlay');
    if (old) old.remove();
    var v = document.getElementById('__rc_veil');
    if (v) v.remove();
    var overlay = document.createElement('div');
    overlay.id = '__rc_overlay';
    overlay.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:rgba(0,0,0,.93);display:flex;align-items:center;justify-content:center;font-family:"gg sans","Segoe UI",system-ui,sans-serif';
    var card = document.createElement('div');
    card.style.cssText = 'background:#313338;border-radius:10px;padding:24px 28px;max-width:400px;text-align:center;box-shadow:0 10px 30px rgba(0,0,0,.6)';
    var title = document.createElement('div');
    title.style.cssText = 'font-size:16px;font-weight:700;color:#f2f3f5;margin-bottom:6px';
    title.textContent = cfg.title;
    var sub = document.createElement('div');
    sub.id = '__rc_sub';
    sub.style.cssText = 'font-size:13px;color:#b5bac1;margin-bottom:16px;line-height:1.5';
    sub.textContent = cfg.subtitle;
    var widget = document.createElement('div');
    widget.id = '__rc_widget';
    widget.style.cssText = 'min-height:78px;display:flex;justify-content:center';
    var err = document.createElement('div');
    err.id = '__rc_err';
    err.style.cssText = 'color:#f87d7d;font-size:12px;margin-top:10px;min-height:14px;white-space:pre-wrap';
    var cancel = document.createElement('div');
    cancel.style.cssText = 'margin-top:14px;font-size:12px;color:#949ba4;cursor:pointer;text-decoration:underline';
    cancel.textContent = 'Отменить вход';
    cancel.onclick = function () { cb('cancel', '1'); };
    card.appendChild(title); card.appendChild(sub); card.appendChild(widget);
    card.appendChild(err); card.appendChild(cancel);
    overlay.appendChild(card);
    document.body.appendChild(overlay);
    var opts = {
      sitekey: cfg.sitekey,
      theme: 'dark',
      callback: function (token) {
        var s = document.getElementById('__rc_sub');
        if (s) s.textContent = 'Проверяем решение…';
        var w = document.getElementById('__rc_widget');
        if (w) w.style.display = 'none';
        cb('captcha', token);
      },
      'error-callback': function (code) {
        var el = document.getElementById('__rc_err');
        if (el) el.textContent = 'Ошибка капчи (' + code + ') — нажми чекбокс ещё раз';
        cb('hcaptcha-err', String(code));
      },
      'expired-callback': function () {
        var el = document.getElementById('__rc_err');
        if (el) el.textContent = 'Проверка истекла — решите ещё раз';
        try { window.hcaptcha.reset(); } catch (e) {}
      }
    };
    if (cfg.rqdata) opts.rqdata = cfg.rqdata;
    if (cfg.invisible) opts.size = 'invisible';
    var id;
    try { id = window.hcaptcha.render(widget, opts); }
    catch (e) {
      err.textContent = 'Не удалось показать капчу: ' + e;
      return;
    }
    if (cfg.invisible) {
      try { window.hcaptcha.execute(id); } catch (e) {}
    }
  }

  window.__rcOnload = function () {
    window.__rcScriptLoading = false;
    var cfg = window.__rcCfg;
    window.__rcCfg = null;
    if (cfg) renderCard(cfg);
  };

  window.__rcAuth = {
    challenge: function (cfg) {
      if (window.hcaptcha && window.hcaptcha.render) { renderCard(cfg); return; }
      window.__rcCfg = cfg;
      if (window.__rcScriptLoading) return;
      window.__rcScriptLoading = true;
      var s = document.createElement('script');
      s.src = 'https://js.hcaptcha.com/1/api.js?render=explicit&onload=__rcOnload';
      s.async = true;
      s.onerror = function () { cb('err', 'hcaptcha-api-load'); };
      (document.head || document.documentElement).appendChild(s);
    },
    request: function (cfg) {
      // Карточка капчи уже висит (ретрай решения) — обновляем её подпись;
      // иначе показываем/обновляем вуаль.
      var card = document.getElementById('__rc_overlay');
      if (card) {
        var sub = document.getElementById('__rc_sub');
        if (sub && cfg.veil) sub.textContent = cfg.veil;
      } else {
        veil(cfg.veil || 'Проверяем…');
      }
      var i = 0;
      function go() {
        return fetch(cfg.urls[i], {
          method: 'POST',
          credentials: 'include',
          headers: headers(cfg),
          body: JSON.stringify(cfg.body)
        }).then(function (r) {
          if (r.status === 404 && i + 1 < cfg.urls.length) { i++; return go(); }
          return r.text().then(function (t) { return { status: r.status, body: t }; });
        });
      }
      go().then(function (r) { cb('http', JSON.stringify(r)); })
        .catch(function (e) { cb('err', 'fetch: ' + e); });
    }
  };
})();
"#;

/// Псевдоним канала событий инжектированного скрипта: (kind, data).
type NavReceiver = mpsc::UnboundedReceiver<(String, String)>;

/// Заголовки и подписи карточки капчи по контексту вызова.
fn titles(context: &str) -> (&'static str, &'static str) {
    match context {
        "qr" => (
            "Подтверждение входа по QR-коду",
            "Discord просит подтвердить, что ты не робот, чтобы завершить \
             вход по QR. Реши капчу — вход продолжится автоматически.",
        ),
        "login" => (
            "Подтверждение входа",
            "Discord просит подтвердить, что ты не робот. Реши капчу — \
             вход продолжится автоматически.",
        ),
        "mfa" => (
            "Подтверждение кода 2FA",
            "Discord просит подтвердить, что ты не робот, прежде чем \
             принять код двухфакторной аутентификации.",
        ),
        _ => (
            "Подтверждение безопасности",
            "Discord просит подтвердить, что ты не робот.",
        ),
    }
}

/// JS-вызов оверлея капчи для конкретного вызова.
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

/// JS-вызов POST-запроса из браузерного контекста discord.com.
/// `captcha_key`/`session_id`/`rqtoken` — заголовки ретрая после решения.
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
        "contextProperties": context_properties,
        "captchaKey": captcha_key,
        "sessionId": session_id,
        "rqtoken": rqtoken,
        "veil": if captcha_key.is_some() {
            "Проверяем решение…"
        } else {
            "Завершаем вход…"
        },
    });
    format!("window.__rcAuth && window.__rcAuth.request({cfg})")
}

/// Открыть окно авторизации на discord.com/login (скрытое — показываем,
/// только когда потребуется капча). Возвращает окно и канал событий
/// инжектированного скрипта (kind, data): ready | http | captcha |
/// hcaptcha-err | err | cancel | closed.
async fn open_auth_window(app: &AppHandle) -> Result<(WebviewWindow, NavReceiver)> {
    // Прошлое окно (незакрытый визит) не должно мешать новому.
    if let Some(stale) = app.get_webview_window(LABEL) {
        let _ = stale.destroy();
        // Webview-ресурсы высвобождаются асинхронно — метка занята до этого.
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    let (tx, rx) = mpsc::unbounded_channel();
    let nav_tx = tx.clone();
    let closed_tx = tx;
    let url: tauri::Url = START_URL
        .parse()
        .context("внутренняя ошибка: URL окна подтверждения")?;

    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title("RustyCord — подтверждение Discord")
        .inner_size(500.0, 560.0)
        .min_inner_size(440.0, 460.0)
        .center()
        .resizable(true)
        .visible(false)
        .always_on_top(true)
        .initialization_script(INIT_SCRIPT)
        .on_navigation(move |url| {
            // Единственный канал связи: блокируем «запрещённую» навигацию
            // и читаем её параметры. Всё остальное (в т.ч. первый заход и
            // редиректы discord.com) — разрешаем.
            let host = url.domain().unwrap_or_default();
            if (host == "discord.com" || host.ends_with(".discord.com"))
                && url.path() == "/__rustycord_cb"
            {
                let mut kind = String::new();
                let mut data = String::new();
                for (k, v) in url.query_pairs() {
                    match k.as_ref() {
                        "s" => kind = v.into_owned(),
                        "d" => data = v.into_owned(),
                        _ => {}
                    }
                }
                let _ = nav_tx.send((kind, data));
                return false;
            }
            true
        })
        .build()
        .context("не удалось открыть окно подтверждения discord.com")?;

    // Крестик окна = отмена (наравне с «Отменить вход» в карточке).
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = closed_tx.send(("closed".to_string(), String::new()));
        }
    });

    Ok((window, rx))
}

/// Показать окно (первая капча): карточка уже отрисована в скрытом
/// документе — пользователь сразу видит её, а не мигающую вуаль.
fn reveal(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Исход «браузерного» запроса.
pub enum BrowserPost {
    /// 2xx — сырой текст тела ответа.
    Ok(String),
    /// Отказ без капчи (форма/429/5xx) — тело для человекочитаемой ошибки.
    Rejected(String),
}

/// POST к API Discord из браузерного окна discord.com с капча-обвязкой.
///
/// Весь цикл живёт в одном контексте: страница сама делает первый POST,
/// при 400+captcha показывает виджет с rqdata из ЭТОГО ЖЕ ответа (rqdata
/// из чужого контекста — «Access Denied»), решение уходит заголовками
/// X-Captcha-Key/-Session-Id/-Rqtoken в повторный POST
/// (≤ CAPTCHA_MAX_ATTEMPTS решений подряд).
///
/// * `context` — qr | login | mfa: подпись карточки капчи;
/// * `urls` — эндпоинты по приоритету: 404 → следующий (v9 → v10);
/// * `body` — JSON-тело запроса;
/// * `context_properties` — b64 X-Context-Properties, если нужен.
pub async fn browser_post(
    app: &AppHandle,
    context: &str,
    urls: &[&str],
    body: &Value,
    context_properties: Option<&str>,
    build: u64,
) -> Result<BrowserPost> {
    let (window, mut rx) = open_auth_window(app).await?;
    let outcome = drive(
        &window,
        &mut rx,
        context,
        urls,
        body,
        context_properties,
        build,
    )
    .await;
    let _ = window.destroy();
    outcome
}

/// Драйвер цикла «POST → капча → ретрай» в окне discord.com.
async fn drive(
    window: &WebviewWindow,
    rx: &mut NavReceiver,
    context: &str,
    urls: &[&str],
    body: &Value,
    context_properties: Option<&str>,
    build: u64,
) -> Result<BrowserPost> {
    let flow = tokio::time::sleep(FLOW_TIMEOUT);
    tokio::pin!(flow);

    let mut challenge: Option<CaptchaChallenge> = None;
    // Решение отправлено, ответ ещё не пришёл: если снова 400+captcha —
    // это отказ решения, такие попытки считаем.
    let mut solution_pending = false;
    let mut rejections: u8 = 0;

    loop {
        tokio::select! {
            _ = &mut flow => bail!("время на подтверждение вышло (5 минут)"),
            nav = rx.recv() => match nav {
                Some((kind, data)) => match kind.as_str() {
                    // Страница готова (в т.ч. перезагрузилась): начинаем
                    // (или начинаем заново) с чистого POST — вызов капчи
                    // рождается в этом же браузерном контексте.
                    "ready" => {
                        solution_pending = false;
                        window
                            .eval(request_js(
                                urls, body, context_properties, build, None, None, None,
                            ))
                            .context("не удалось отправить запрос (eval)")?;
                    }
                    "http" => {
                        let v: Value = serde_json::from_str(&data)
                            .context("странный ответ страницы на запрос")?;
                        let status = v["status"].as_u64().unwrap_or(0);
                        let text = v["body"].as_str().unwrap_or_default().to_string();
                        if (200..300).contains(&status) {
                            return Ok(BrowserPost::Ok(text));
                        }
                        let parsed: Value =
                            serde_json::from_str(&text).unwrap_or(Value::Null);
                        if status == 400 {
                            if solution_pending {
                                rejections += 1;
                            }
                            solution_pending = false;
                            if let Some(next) = CaptchaChallenge::from_body(&parsed) {
                                if rejections >= CAPTCHA_MAX_ATTEMPTS {
                                    bail!(
                                        "капча: Discord отклонил решение \
                                         {CAPTCHA_MAX_ATTEMPTS} раз подряд"
                                    );
                                }
                                tracing::warn!(
                                    "captcha-webview: HTTP 400, показываем капчу \
                                     (отказов {rejections}, invisible={})",
                                    next.invisible
                                );
                                let js = challenge_js(&next, context);
                                challenge = Some(next);
                                window
                                    .eval(&js)
                                    .context("не удалось показать капчу (eval)")?;
                                reveal(window);
                                continue;
                            }
                        }
                        let snippet: String = text.chars().take(300).collect();
                        tracing::warn!("captcha-webview: HTTP {status} — {snippet}");
                        return Ok(BrowserPost::Rejected(text));
                    }
                    "captcha" => {
                        // Решение получено: повторяем POST с X-Captcha-*.
                        let Some(ch) = challenge.as_ref() else {
                            // Решение без живого вызова (перезагрузка?) —
                            // начинаем цикл заново чистым POST.
                            solution_pending = false;
                            window
                                .eval(request_js(
                                    urls, body, context_properties, build, None, None, None,
                                ))
                                .context("не удалось повторить запрос (eval)")?;
                            continue;
                        };
                        solution_pending = true;
                        window
                            .eval(request_js(
                                urls,
                                body,
                                context_properties,
                                build,
                                Some(&data),
                                ch.session_id.as_deref(),
                                ch.rqtoken.as_deref(),
                            ))
                            .context("не удалось повторить запрос (eval)")?;
                    }
                    // Диагностика: код ошибки виджета в лог, поток жив.
                    "hcaptcha-err" => {
                        tracing::warn!("captcha-webview: ошибка виджета hCaptcha — {data}");
                    }
                    "err" => bail!("страница подтверждения: {data}"),
                    "cancel" => bail!("отменено пользователем"),
                    "closed" => bail!("окно подтверждения закрыто"),
                    _ => {}
                },
                None => bail!("окно подтверждения закрыто"),
            },
        }
    }
}
