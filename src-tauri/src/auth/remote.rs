//! Remote auth (QR-код), протокол v2 — wss://remote-auth-gateway.discord.gg/?v=2
//!
//! Протокол, которым официальный клиент логинится через телефон.
//! Спецификация: docs.discord.food «Remote Authentication (Desktop)»;
//! проверен вживую пробником scripts/qr-probe.mjs.
//!
//! Флоу:
//!  1. hello (heartbeat_interval, timeout_ms) — сразу шлём op:init с нашим
//!     RSA-2048 публичным ключом (SPKI DER → base64);
//!  2. nonce_proof (encrypted_nonce) — расшифровываем и ЭХОМ возвращаем
//!     nonce в base64URL без «=» (v2: никаких HMAC-proof);
//!  3. pending_remote_init (fingerprint = SHA-256 нашего ключа) — рисуем QR
//!     «https://discord.com/ra/{fingerprint}»; fingerprint сверяем с хэшем
//!     своего ключа (защита от подмены);
//!  4. heartbeat — шлём каждые hello.heartbeat_interval мс, сервер отвечает
//!     heartbeat_ack; нет ACK — соединение «зомби», обрываем;
//!  5. телефон сканирует QR → pending_ticket (зашифрованный профиль
//!     «id:discriminator:avatar:username») → «отсканировал такой-то юзер»;
//!  6. pending_login (ticket) — REST POST /api/v9/users/@me/remote-auth/login
//!     (v9 — как официальный веб-клиент; v10 запасной) → encrypted_token
//!     → RSA-OAEP-SHA256 → пользовательский токен;
//!  7. cancel / close 4003 — отмена на телефоне / QR истёк.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use super::captcha;
use super::captcha_webview;
use super::{nonce_echo, RemoteAuthKeys};
use crate::superprops::BROWSER_UA;
use crate::wsutil;

const REMOTE_AUTH_URL: &str = "wss://remote-auth-gateway.discord.gg/?v=2";
/// Обмен QR-тикетa на токен (docs.discord.food, Endpoints).
/// Рабочая версия — **v9**: её используют официальный веб-клиент и
/// battle-tested обёртка discord-remote-auth (npm). v10 на живом тикете
/// отвечал 400 без тела (лог 18:33:09) — оставлен как запасной эндпоинт.
const TICKET_EXCHANGE_URLS: [&str; 2] = [
    "https://discord.com/api/v9/users/@me/remote-auth/login",
    "https://discord.com/api/v10/users/@me/remote-auth/login",
];

/// Собрать компактный QR-SVG (только чёрные модули, quiet zone 4).
fn qr_svg(data: &str) -> Result<String> {
    let code = qrcode::QrCode::new(data.as_bytes()).context("QR encode")?;
    let width = code.width();
    let modules = code.to_colors(); // row-major; Color::Dark = модуль «включен»
    let quiet = 4i32;
    let total = (width as i32 + quiet * 2) as f64;
    let mut path = String::with_capacity(4096);
    for y in 0..width {
        for x in 0..width {
            if matches!(modules[y * width + x], qrcode::Color::Dark) {
                path.push_str(&format!(
                    "M{},{}h1v1h-1z",
                    x as i32 + quiet,
                    y as i32 + quiet
                ));
            }
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {t} {t}\" shape-rendering=\"crispEdges\">\
         <rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\
         <path d=\"{path}\" fill=\"#060607\"/></svg>",
        t = total,
        path = path
    ))
}

/// Обменять ticket на токен: POST {ticket} → encrypted_token → RSA → токен.
/// Это финальный шаг v2: сам WebSocket токен больше не приносит.
/// `build` — client_build_number (префетч в начале QR-сессии, см.
/// run_remote_auth): тянуть его здесь нельзя — обмен и так самый
/// уязвимый шаг входа, лишние секунды задержки не нужны.
///
/// Тикет **одноразовый**: пока не было 2xx, его можно переотправлять
/// (в т.ч. с решённой капчей). Потому:
///  * быстрый путь — прямой POST по двум URL (v9 → v10): у «чистых»
///    IP/контекстов капчи не будет вовсе;
///  * капча (400 + captcha_key) — весь обмен уезжает в окно discord.com
///    (captcha_webview::browser_post): rqdata Enterprise привязан к
///    контексту, поэтому POST, породивший вызов, и виджет обязаны жить
///    в одном браузерном окне (≤ CAPTCHA_MAX_ATTEMPTS решений), тикет
///    при не-2xx не тратится.
async fn exchange_ticket(
    keys: &RemoteAuthKeys,
    ticket: &str,
    build: u64,
    app: &tauri::AppHandle,
) -> Result<String> {
    if ticket.len() < 20 {
        bail!(
            "подозрительный ticket (длина {} — ожидаем строку вида \
             «id.подпись.секрет»), обмен не выполняем",
            ticket.len()
        );
    }
    // Cookie-store включён сознательно: официальный веб-клиент шлёт этот
    // POST с куками discord.com (__dcfduid/__sdcfid) — они участвуют в
    // риск-скоринге, без них шанс упереться в капчу заметно выше.
    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(Duration::from_secs(15))
        .cookie_store(true)
        .build()
        .context("не удалось создать HTTP-клиент")?;
    // Сеем куки: лёгкий GET страницы логина (результат не важен).
    let _ = client.get("https://discord.com/login").send().await;

    let mut last: Option<String> = None;
    'urls: for url in TICKET_EXCHANGE_URLS {
        match exchange_once(&client, keys, url, ticket, build).await {
            ExchangeOutcome::Token(token) => return Ok(token),
            ExchangeOutcome::Fatal(msg) => {
                bail!("{msg}");
            }
            ExchangeOutcome::Rejected(msg) => {
                tracing::warn!("remote-auth: {url} → {msg}; пробуем запасной эндпоинт");
                last = Some(msg);
                continue 'urls;
            }
            ExchangeOutcome::Captcha => {
                tracing::warn!(
                    "remote-auth: {url} требует капчу — обмен уходит в окно \
                     discord.com (rqdata рождается и решается в одном \
                     браузерном контексте)"
                );
                // Полный цикл «POST → капча → ретрай» внутри окна на
                // discord.com: браузерный контекст, правильный origin для
                // sitekey и консистентный rqdata. Ответ расшифровываем
                // своим ключом: только у нас приватная половина RSA.
                let outcome = captcha_webview::browser_post(
                    app,
                    "qr",
                    &[
                        "/api/v9/users/@me/remote-auth/login",
                        "/api/v10/users/@me/remote-auth/login",
                    ],
                    &json!({ "ticket": ticket }),
                    None,
                    build,
                )
                .await
                .map_err(|e| anyhow::anyhow!("капча-обмен: {e:#}"))?;
                let raw = match outcome {
                    captcha_webview::BrowserPost::Ok(text) => text,
                    captcha_webview::BrowserPost::Rejected(raw) => {
                        let snippet: String = raw.chars().take(300).collect();
                        bail!("обмен после капчи отклонён: {snippet}");
                    }
                };
                let body: Value = serde_json::from_str(&raw).with_context(|| {
                    format!(
                        "2xx-ответ не JSON: {}",
                        raw.chars().take(200).collect::<String>()
                    )
                })?;
                let enc = body["encrypted_token"]
                    .as_str()
                    .ok_or_else(|| anyhow!("в 2xx-ответе нет encrypted_token"))?;
                let bytes = keys
                    .decrypt(enc)
                    .context("RSA-расшифровка токена после капчи")?;
                return String::from_utf8(bytes).context("токен после капчи — не UTF-8");
            }
        }
    }
    bail!(
        "обмен ticket не удался (обе версии API): {}",
        last.unwrap_or_else(|| "нет ответа".to_string())
    )
}

/// Исход одной попытки обмена.
enum ExchangeOutcome {
    /// Токен получен и расшифрован.
    Token(String),
    /// Сервер требует капчу — тикет не потрачен: весь обмен уезжает в окно
    /// discord.com, которое само повторит POST и решит капчу.
    Captcha,
    /// Другой отказ — тикет не потрачен, можно пробовать другой URL.
    Rejected(String),
    /// Был 2xx, но токен из ответа не извлечён — тикет уже сгорел.
    Fatal(String),
}

/// Одна попытка обмена: полный набор браузерных заголовков (как у
/// официального веб-клиента и discord-remote-auth: Referer, sec-fetch-*,
/// Accept-Language, X-Super-Properties) + диагностика сырым телом ответа.
async fn exchange_once(
    client: &reqwest::Client,
    keys: &RemoteAuthKeys,
    url: &str,
    ticket: &str,
    build: u64,
) -> ExchangeOutcome {
    let req = client
        .post(url)
        .header("Origin", "https://discord.com")
        .header("Referer", "https://discord.com/login")
        .header("Accept-Language", "ru-RU,ru;q=0.9")
        .header("X-Discord-Locale", "ru-RU")
        .header(
            "X-Super-Properties",
            crate::superprops::super_properties(build),
        )
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .json(&json!({ "ticket": ticket }));
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return ExchangeOutcome::Rejected(format!("сеть: {e}")),
    };
    let status = resp.status();
    // Читаем тело как текст: раньше парсили сразу в JSON и теряли сырой
    // ответ — «нет описания» в логе скрывал реальную причину 400.
    let raw = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Сначала — капча (400 + captcha_key): тикет не потрачен, решаем
        // через UI и повторяем (docs.discord.food/topics/captcha-handling).
        if let Ok(body) = serde_json::from_str::<Value>(&raw) {
            if captcha::CaptchaChallenge::from_body(&body).is_some() {
                return ExchangeOutcome::Captcha;
            }
        }
        let snippet: String = raw.chars().take(300).collect();
        return ExchangeOutcome::Rejected(format!(
            "HTTP {status} (ticket len {}) — ответ: {}",
            ticket.len(),
            if snippet.is_empty() {
                "<пустое тело>".to_string()
            } else {
                snippet
            }
        ));
    }
    // 2xx: тикет потрачен. Всё, что ниже, — Fatal (повтор с тем же тикетом
    // бессмыслен).
    let body: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return ExchangeOutcome::Fatal(format!(
                "ответ 2xx не JSON ({e}): {}",
                raw.chars().take(200).collect::<String>()
            ))
        }
    };
    let Some(enc) = body["encrypted_token"].as_str() else {
        return ExchangeOutcome::Fatal(format!(
            "в 2xx-ответе нет encrypted_token: {}",
            raw.chars().take(200).collect::<String>()
        ));
    };
    let bytes = match keys.decrypt(enc) {
        Ok(b) => b,
        Err(e) => return ExchangeOutcome::Fatal(format!("RSA-расшифровка: {e}")),
    };
    match String::from_utf8(bytes) {
        Ok(token) => ExchangeOutcome::Token(token),
        Err(_) => ExchangeOutcome::Fatal("токен не UTF-8".to_string()),
    }
}

/// Одна QR-сессия. Отменяется через `cancel` (пользователь закрыл экран логина).
/// Капча (если риск-скоринг потребует) решается в отдельном окне discord.com
/// (см. auth/captcha_webview.rs).
pub async fn run_remote_auth(app: tauri::AppHandle, cancel: CancellationToken) -> Result<()> {
    let emit = |stage: &str, extra: Value| {
        // json! не поддерживает spread — мерджим объекты вручную.
        let mut payload = extra;
        if !payload.is_object() {
            payload = json!({});
        }
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("stage".to_string(), json!(stage));
        }
        let _ = app.emit("discord://qr", payload);
    };

    emit(
        "connecting",
        json!({ "message": "Подключаемся к Discord…" }),
    );

    let keys = RemoteAuthKeys::generate().context("не удалось сгенерировать RSA-ключи")?;
    // Префетч client_build_number для X-Super-Properties обмена тикета:
    // тянем ПАРАЛЛЕЛЬНО рукопожатию WS — пока пользователь достаёт телефон
    // (десятки секунд), номер давно готов; на обмене тикета лишних секунд
    // уже не будет.
    let build_handle = tokio::spawn(crate::superprops::fetch_build_number());

    let mut request = REMOTE_AUTH_URL.into_client_request()?;
    // Origin обязателен (docs): без него шлюз отклоняет соединение.
    request.headers_mut().insert(
        "User-Agent",
        reqwest::header::HeaderValue::from_static(BROWSER_UA),
    );
    request.headers_mut().insert(
        "Origin",
        reqwest::header::HeaderValue::from_static("https://discord.com"),
    );

    let (ws, _) = connect_async(request)
        .await
        .context("не удалось открыть WebSocket до remote-auth-gateway")?;
    let (mut writer, mut reader) = wsutil::split(ws);

    // ---- op:init: публикуем свой ключ ----
    wsutil::send_json(
        &mut writer,
        &json!({
            "op": "init",
            "encoded_public_key": keys.public_b64
        }),
    )
    .await?;

    // Heartbeat: интервал приходит в hello; тикаем сами, ждём heartbeat_ack.
    let mut hb = tokio::time::interval(Duration::from_secs(41));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    hb.tick().await; // проглотить немедленный тик
    let mut hb_on = false;
    let mut hb_acked = true;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                // Вежливо отменяем сессию на сервере, если сокет ещё жив.
                let _ = wsutil::send_json(&mut writer, &json!({"op": "cancel"})).await;
                let _ = writer.send(Message::Close(None)).await;
                return Ok(());
            }

            _ = hb.tick(), if hb_on => {
                if !hb_acked {
                    bail!("remote-auth: нет heartbeat_ack — соединение потеряно");
                }
                wsutil::send_json(&mut writer, &json!({"op": "heartbeat"})).await?;
                hb_acked = false;
            }

            msg = reader.next() => {
                let Some(msg) = msg else { bail!("remote-auth: соединение закрыто") };
                let msg = msg.context("remote-auth ws ошибка")?;

                // Close-кадр: 4003 = QR истёк, остальное — ошибка сессии.
                if let Message::Close(frame) = &msg {
                    let code = frame.as_ref().map(|f| u16::from(f.code));
                    match code {
                        Some(4003) => {
                            let _ = writer.send(Message::Close(None)).await;
                            emit("timeout", json!({ "message": "QR-код истёк — попробуй снова" }));
                            return Ok(());
                        }
                        other => bail!("remote-auth: сервер закрыл соединение (код {other:?})"),
                    }
                }
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).context("remote-auth: битый JSON")?;
                let op = v["op"].as_str().unwrap_or_default().to_string();

                match op.as_str() {
                    "hello" => {
                        // Узнали интервал — запускаем свои heartbeat-тики.
                        if let Some(interval) = v["heartbeat_interval"].as_u64() {
                            hb = tokio::time::interval(Duration::from_millis(
                                interval.clamp(5_000, 120_000),
                            ));
                            hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                            hb.tick().await;
                            hb_on = true;
                        }
                    }
                    "heartbeat_ack" => {
                        hb_acked = true;
                    }
                    "nonce_proof" => {
                        let enc = v["encrypted_nonce"]
                            .as_str()
                            .context("в nonce_proof нет encrypted_nonce")?;
                        let echo = nonce_echo(enc, &keys)
                            .context("не удалось расшифровать nonce сервера")?;
                        wsutil::send_json(
                            &mut writer,
                            &json!({
                                "op": "nonce_proof",
                                "nonce": echo
                            }),
                        )
                        .await?;
                    }
                    "pending_remote_init" => {
                        let fingerprint =
                            v["fingerprint"].as_str().unwrap_or_default().to_string();
                        if fingerprint.is_empty() {
                            bail!("pending_remote_init без fingerprint");
                        }
                        // Безопасность: fingerprint = SHA-256 нашего ключа.
                        if fingerprint != keys.expected_fingerprint() {
                            bail!("fingerprint не совпал с SHA-256 нашего ключа — возможна подмена");
                        }
                        let qr_data = format!("https://discord.com/ra/{fingerprint}");
                        let svg = qr_svg(&qr_data)?;
                        emit("fingerprint", json!({ "svg": svg, "fingerprint": fingerprint }));
                    }
                    "pending_ticket" => {
                        // Телефон просканировал QR: показываем, кто это был.
                        let enc = v["encrypted_user_payload"]
                            .as_str()
                            .context("в pending_ticket нет encrypted_user_payload")?;
                        let payload = keys
                            .decrypt(enc)
                            .context("не удалось расшифровать профиль пользователя")
                            .and_then(|b| {
                                String::from_utf8(b).context("профиль не UTF-8")
                            })?;
                        // Формат: id:discriminator:avatar:username
                        let parts: Vec<&str> = payload.splitn(4, ':').collect();
                        let (user_id, username) = if parts.len() == 4 {
                            (parts[0], parts[3])
                        } else {
                            ("", "")
                        };
                        emit(
                            "scanned",
                            json!({ "username": username, "userId": user_id }),
                        );
                    }
                    "pending_login" => {
                        // Телефон подтвердил вход: меняем ticket на токен.
                        let ticket = v["ticket"]
                            .as_str()
                            .context("в pending_login нет ticket")?;
                        tracing::info!(
                            "remote-auth: pending_login, ticket len {}",
                            ticket.len()
                        );
                        let build = build_handle
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or(crate::superprops::FALLBACK_BUILD_NUMBER);
                        let token = exchange_ticket(&keys, ticket, build, &app).await?;
                        let _ = writer.send(Message::Close(None)).await;
                        emit("done", json!({ "token": token }));
                        return Ok(());
                    }
                    "cancel" => {
                        let _ = writer.send(Message::Close(None)).await;
                        emit("cancelled", json!({ "message": "Вход отменён на телефоне" }));
                        return Ok(());
                    }
                    // v1-легаси: в v2 таймаут приходит Close-кадром 4003.
                    "timeout" => {
                        let _ = writer.send(Message::Close(None)).await;
                        emit("timeout", json!({ "message": "QR-код истёк — попробуй снова" }));
                        return Ok(());
                    }
                    other => {
                        tracing::debug!("remote-auth: игнорируем op «{other}»");
                    }
                }
            }
        }
    }
}
