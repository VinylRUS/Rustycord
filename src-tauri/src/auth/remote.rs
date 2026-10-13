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
//!  6. pending_login (ticket) — REST POST /users/@me/remote-auth/login
//!     → encrypted_token → RSA-OAEP-SHA256 → пользовательский токен;
//!  7. cancel / close 4003 — отмена на телефоне / QR истёк.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use super::{nonce_echo, RemoteAuthKeys};
use crate::superprops::BROWSER_UA;
use crate::wsutil;

const REMOTE_AUTH_URL: &str = "wss://remote-auth-gateway.discord.gg/?v=2";
/// Обмен QR-тикетa на токен (docs.discord.food, Endpoints).
const TICKET_EXCHANGE_URL: &str = "https://discord.com/api/v10/users/@me/remote-auth/login";

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
async fn exchange_ticket(keys: &RemoteAuthKeys, ticket: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(Duration::from_secs(15))
        .build()
        .context("не удалось создать HTTP-клиент")?;
    let resp = client
        .post(TICKET_EXCHANGE_URL)
        .header("Origin", "https://discord.com")
        .json(&json!({ "ticket": ticket }))
        .send()
        .await
        .context("не удалось обменять ticket (POST remote-auth/login)")?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!(
            "обмен ticket: HTTP {status} — {}",
            body["message"].as_str().unwrap_or("нет описания")
        );
    }
    let enc = body["encrypted_token"]
        .as_str()
        .context("в ответе нет encrypted_token")?;
    let bytes = keys.decrypt(enc).context("не удалось расшифровать токен")?;
    String::from_utf8(bytes).context("токен не UTF-8")
}

/// Одна QR-сессия. Отменяется через `cancel` (пользователь закрыл экран логина).
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
                        let token = exchange_ticket(&keys, ticket).await?;
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
