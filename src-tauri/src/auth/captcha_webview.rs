//! Капча hCaptcha в отдельном окне на discord.com.
//!
//! Почему не в окне приложения: sitekey Discord (`a9b5fb07-…`) привязан
//! к домену — hCaptcha Enterprise проверяет `host` страницы, где стоит
//! виджет. В Tauri-окне origin `tauri://localhost`, и getcaptcha молча
//! отклоняет вызов (виджет рисуется с красным «Incorrect domain!»,
//! клик даёт error-callback `network-error`). Проверено экспериментально
//! на тестовой странице.
//!
//! Решение — окно-WebView на https://discord.com/login (настоящий origin
//! discord.com + куки браузера + реальный fingerprint: риск-скоринг видит
//! подлинный веб-клиент, а не reqwest). В страницу инжектируется скрипт
//! (initialization_script, работает на любой странице discord.com):
//!  * `__rcAuth.challenge(cfg)` — полноэкранный оверлей с виджетом hCaptcha
//!    (sitekey + rqdata из вызова Discord);
//!  * `__rcAuth.exchange(cfg)` — fetch POST /api/v9/users/@me/remote-auth/login
//!    с заголовками X-Captcha-Key/-Session-Id/-Rqtoken — тот самый обмен
//!    QR-тикета, но из браузерного контекста.
//!
//! Обратная связь — через перехват навигации: инжектированный JS делает
//! `location.href = '/__rustycord_cb?s=<kind>&d=<data>'`, а on_navigation
//! блокирует переход и отдаёт данные в Rust-канал. CSP discord.com это
//! позволяет: js.hcaptcha.com/newassets.hcaptcha.com разрешены в
//! script-src/frame-src/style-src (страница логина сама показывает капчу).

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::mpsc;

use super::captcha::{CaptchaChallenge, CAPTCHA_MAX_ATTEMPTS};
use crate::superprops;

/// Метка окна авторизации (одно на приложение).
const LABEL: &str = "rustycord-auth";
/// Стартовая страница: origin discord.com обязателен для капчи, а куки,
/// которые она посеет, улучшают риск-скоринг последующих запросов.
const START_URL: &str = "https://discord.com/login";
/// Общий бюджет на весь визит окна: загрузка страницы + решение капчи.
/// Первый `ready` внутри этого окна обязан прийти за десятки секунд,
/// отдельного таймаута загрузки не нужно.
const FLOW_TIMEOUT: Duration = Duration::from_secs(300);

/// Инжектируется в окно авторизации до скриптов страницы (на каждый заход).
const INIT_SCRIPT: &str = r#"
(function () {
  'use strict';
  if (window.__rcAuthInstalled) return;
  window.__rcAuthInstalled = true;
  function cb(kind, data) {
    location.href = '/__rustycord_cb?s=' + encodeURIComponent(kind) + '&d=' + encodeURIComponent(data);
  }
  function ping() { cb('ready', '1'); }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', ping);
  else ping();

  function renderCard(cfg) {
    var old = document.getElementById('__rc_overlay');
    if (old) old.remove();
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
      callback: function (token) { cb('captcha', token); },
      'error-callback': function (code) {
        var el = document.getElementById('__rc_err');
        if (el) el.textContent = 'Ошибка капчи (' + code + ') — нажми чекбокс ещё раз';
      },
      'expired-callback': function () {
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
      document.head.appendChild(s);
    },
    exchange: function (cfg) {
      var note = document.getElementById('__rc_err');
      if (note) note.textContent = '';
      var sub = document.getElementById('__rc_sub');
      if (sub) sub.textContent = 'Проверяем решение…';
      var h = {
        'Content-Type': 'application/json',
        'X-Discord-Locale': 'ru-RU',
        'X-Super-Properties': cfg.xsp
      };
      if (cfg.captchaKey) h['X-Captcha-Key'] = cfg.captchaKey;
      if (cfg.sessionId) h['X-Captcha-Session-Id'] = cfg.sessionId;
      if (cfg.rqtoken) h['X-Captcha-Rqtoken'] = cfg.rqtoken;
      fetch('/api/v9/users/@me/remote-auth/login', {
        method: 'POST',
        credentials: 'include',
        headers: h,
        body: JSON.stringify({ ticket: cfg.ticket })
      })
        .then(function (r) { return r.text().then(function (t) { return { status: r.status, body: t }; }); })
        .then(function (r) { cb('exchange', JSON.stringify(r)); })
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

/// JS-вызов обмена тикета (fetch из браузерного контекста discord.com).
fn exchange_js(
    ticket: &str,
    xsp: &str,
    captcha_key: &str,
    session_id: Option<&str>,
    rqtoken: Option<&str>,
) -> String {
    let cfg = json!({
        "ticket": ticket,
        "xsp": xsp,
        "captchaKey": captcha_key,
        "sessionId": session_id,
        "rqtoken": rqtoken,
    });
    format!("window.__rcAuth && window.__rcAuth.exchange({cfg})")
}

/// Открыть окно авторизации на discord.com/login. Возвращает окно и канал
/// событий инжектированного скрипта (kind, data): ready | captcha | exchange
/// | err | cancel | closed.
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
        .initialization_script(INIT_SCRIPT)
        .on_navigation(move |url| {
            // Единственный канал связи: блокируем «запрещённую» навигацию
            // и читаем её параметры. Всё остальное (в т.ч. первый заход и
            // редиректы discord.com) — разрешаем.
            if url.domain() == Some("discord.com") && url.path() == "/__rustycord_cb" {
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

/// Решить капчу для любого запроса (вход email/2FA): показать оверлей в окне
/// discord.com и дождаться токена hCaptcha. Токен вызывающий повторно шлёт
/// заголовками X-Captcha-* (см. auth::captcha::SolvedCaptcha).
pub async fn solve_captcha(
    app: &AppHandle,
    context: &str,
    challenge: &CaptchaChallenge,
) -> Result<String> {
    let (window, mut rx) = open_auth_window(app).await?;
    let outcome = drive_solve(&window, &mut rx, context, challenge).await;
    let _ = window.destroy();
    outcome
}

/// Драйвер «показать капчу → токен» без владения окном (для переиспользования
/// в обмене тикета).
async fn drive_solve(
    window: &WebviewWindow,
    rx: &mut NavReceiver,
    context: &str,
    challenge: &CaptchaChallenge,
) -> Result<String> {
    let flow = tokio::time::sleep(FLOW_TIMEOUT);
    tokio::pin!(flow);
    // Оверлей ставится после готовности страницы; при перезагрузке
    // (ready приходит снова) — перевзводим, виджет честно рисуется заново.
    let call = challenge_js(challenge, context);
    loop {
        tokio::select! {
            _ = &mut flow => bail!("время на подтверждение вышло (5 минут)"),
            nav = rx.recv() => match nav {
                Some((kind, data)) => match kind.as_str() {
                    "ready" => {
                        window
                            .eval(&call)
                            .context("не удалось показать капчу (eval)")?;
                    }
                    "captcha" => return Ok(data),
                    "err" => bail!("страница подтверждения: {data}"),
                    "cancel" => bail!("отменено пользователем"),
                    "closed" => bail!("окно подтверждения закрыто"),
                    _ => {} // exchange здесь не бывает
                },
                None => bail!("окно подтверждения закрыто"),
            },
        }
    }
}

/// Обмен QR-тикета, когда Discord потребовал капчу: решить её в окне
/// discord.com и тут же, из той же страницы, повторить POST обмена
/// (origin + куки + fingerprint настоящего браузера). Ответ 2xx с новым
/// вызовом капчи рендерит следующую попытку — не более CAPTCHA_MAX_ATTEMPTS.
///
/// Возвращает `encrypted_token` (расшифровка — у вызывающего: только у него
/// приватный RSA-ключ сессии).
pub async fn exchange_ticket(
    app: &AppHandle,
    ticket: &str,
    build: u64,
    first_challenge: &CaptchaChallenge,
) -> Result<String> {
    let (window, mut rx) = open_auth_window(app).await?;
    let outcome = drive_exchange(&window, &mut rx, ticket, build, first_challenge).await;
    let _ = window.destroy();
    outcome
}

async fn drive_exchange(
    window: &WebviewWindow,
    rx: &mut NavReceiver,
    ticket: &str,
    build: u64,
    first_challenge: &CaptchaChallenge,
) -> Result<String> {
    let xsp = superprops::super_properties(build);
    let flow = tokio::time::sleep(FLOW_TIMEOUT);
    tokio::pin!(flow);

    let mut challenge = first_challenge.clone();
    let mut attempts: u8 = 0;

    loop {
        tokio::select! {
            _ = &mut flow => bail!("время на подтверждение вышло (5 минут)"),
            nav = rx.recv() => match nav {
                Some((kind, data)) => match kind.as_str() {
                    "ready" => {
                        window
                            .eval(challenge_js(&challenge, "qr"))
                            .context("не удалось показать капчу (eval)")?;
                    }
                    "captcha" => {
                        // Пользователь прошёл капчу: ретрай-фетч уходит из
                        // той же страницы — Discord видит подлинный контекст.
                        let js = exchange_js(
                            ticket,
                            &xsp,
                            &data,
                            challenge.session_id.as_deref(),
                            challenge.rqtoken.as_deref(),
                        );
                        window
                            .eval(&js)
                            .context("не удалось повторить обмен (eval)")?;
                    }
                    "exchange" => {
                        let v: Value = serde_json::from_str(&data)
                            .context("странный ответ страницы на обмен")?;
                        let status = v["status"].as_u64().unwrap_or(0);
                        let body = v["body"].as_str().unwrap_or_default().to_string();
                        if (200..300).contains(&status) {
                            let parsed: Value = serde_json::from_str(&body)
                                .with_context(|| format!("2xx-ответ не JSON: {}", &body[..body.len().min(200)]))?;
                            let enc = parsed["encrypted_token"]
                                .as_str()
                                .ok_or_else(|| anyhow!("в 2xx-ответе нет encrypted_token"))?;
                            return Ok(enc.to_string());
                        }
                        // 400 + новый вызов капчи → следующая попытка.
                        if status == 400 {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&body) {
                                if let Some(next) = CaptchaChallenge::from_body(&parsed) {
                                    attempts += 1;
                                    if attempts >= CAPTCHA_MAX_ATTEMPTS {
                                        bail!(
                                            "капча: Discord отклонил решение \
                                             {CAPTCHA_MAX_ATTEMPTS} раз подряд"
                                        );
                                    }
                                    challenge = next;
                                    window
                                        .eval(challenge_js(&challenge, "qr"))
                                        .context("не удалось показать следующую капчу")?;
                                    continue;
                                }
                            }
                        }
                        let snippet: String =
                            body.chars().take(300).collect();
                        bail!("обмен после капчи: HTTP {status} — {snippet}");
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
