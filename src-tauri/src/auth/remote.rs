//! Remote auth (QR-код) — протокол, которым официальный десктоп-клиент
//! логинится через телефон: wss://remote-auth-gateway.discord.gg/?v=2
//!
//! Флоу:
//!  1. генерируем RSA-2048 пару, шлём публичный ключ (op:init);
//!  2. сервер присылает зашифрованный nonce (op:nonce_proof) — доказываем
//!     владение приватным ключом через HMAC-proof (op:pending_verify);
//!  3. op:pending_remote_auth → fingerprint → рисуем QR
//!     «https://discord.com/ra/{fingerprint}»;
//!  4. человек подтверждает в приложении → op:pending_ticket (просканировано);
//!  5. op:pending_login → encrypted_token, расшифровываем своим ключом.
//!
//! Отмена/таймаут: op:cancel / op:timeout. Heartbeat — сервер шлёт op:heartbeat,
//! мы эхом отвечаем тем же кадром (плюс поддерживаем поле interval).

use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use super::{nonce_proof, RemoteAuthKeys};
use crate::superprops::BROWSER_UA;
use crate::wsutil;

const REMOTE_AUTH_URL: &str = "wss://remote-auth-gateway.discord.gg/?v=2";

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

    let keys = RemoteAuthKeys::generate().context("не удалось сгенерировать RSA-ключи")?;

    let mut request = REMOTE_AUTH_URL.into_client_request()?;
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

    // heartbeat: отвечаем эхом; interval-поле (если придёт) включает свой таймер.
    let mut hb_own = tokio::time::interval(Duration::from_secs(10));
    hb_own.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    hb_own.tick().await; // проглотить немедленный тик
    let mut hb_enabled = false;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                // Вежливо отменяем сессию на сервере, если сокет ещё жив.
                let _ = wsutil::send_json(&mut writer, &json!({"op": "cancel"})).await;
                let _ = writer.send(Message::Close(None)).await;
                return Ok(());
            }

            _ = hb_own.tick(), if hb_enabled => {
                wsutil::send_json(&mut writer, &json!({"op": "heartbeat"})).await?;
            }

            msg = reader.next() => {
                let Some(msg) = msg else { bail!("remote-auth: соединение закрыто") };
                let msg = msg.context("remote-auth ws ошибка")?;
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).context("remote-auth: битый JSON")?;
                let op = v["op"].as_str().unwrap_or_default().to_string();

                match op.as_str() {
                    "heartbeat" => {
                        // Эхо-ответ; заодно могли прислать интервал.
                        if let Some(interval) = v["interval"].as_u64() {
                            hb_own = tokio::time::interval(Duration::from_millis(interval.clamp(2000, 60_000)));
                            hb_own.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                            hb_own.tick().await;
                            hb_enabled = true;
                        }
                        wsutil::send_json(&mut writer, &json!({"op": "heartbeat"})).await?;
                    }
                    "nonce_proof" => {
                        let nonce = v["nonce"].as_str().unwrap_or_default();
                        let proof = nonce_proof(nonce, &keys)
                            .context("nonce_proof отклонён (ключ/формат)")?;
                        wsutil::send_json(&mut writer, &json!({
                            "op": "pending_verify",
                            "proof": proof
                        }))
                        .await?;
                    }
                    "pending_remote_auth" => {
                        let fingerprint = v["fingerprint"].as_str().unwrap_or_default().to_string();
                        let qr_data = format!("https://discord.com/ra/{fingerprint}");
                        let svg = qr_svg(&qr_data)?;
                        emit("fingerprint", json!({ "svg": svg, "fingerprint": fingerprint }));
                    }
                    "pending_ticket" => {
                        // Телефон просканировал QR — ждём подтверждения.
                        emit("scanned", json!({}));
                    }
                    "pending_login" => {
                        let enc = v["encrypted_token"].as_str().unwrap_or_default();
                        let token = keys
                            .decrypt(enc)
                            .context("не удалось расшифровать токен")
                            .and_then(|bytes| {
                                String::from_utf8(bytes).context("токен не UTF-8")
                            })?;
                        let _ = writer.send(Message::Close(None)).await;
                        emit("done", json!({ "token": token }));
                        return Ok(());
                    }
                    "cancel" => {
                        let _ = writer.send(Message::Close(None)).await;
                        emit("cancelled", json!({ "message": "Вход отменён на телефоне" }));
                        return Ok(());
                    }
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
