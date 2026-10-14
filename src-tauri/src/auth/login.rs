//! POST /auth/login — вход email+паролем, ровно как форма на discord.com/login.
//!
//! Исходы, как у оригинала:
//!  * `{token}` — готово;
//!  * `{mfa: true, ticket}` — нужен код 2FA (TOTP), добивается вторым запросом
//!    POST /auth/mfa/totp;
//!  * 400 с полем `captcha_key` — Discord требует hCaptcha: rqdata Enterprise
//!    привязан к контексту сессии, поэтому весь запрос уезжает в окно
//!    discord.com (auth/captcha_webview.rs) и повторяется там с нуля —
//!    вызов капчи рождается и решается в одном браузерном контексте
//!    (docs.discord.food/topics/captcha-handling).

use serde_json::{json, Value};

use crate::auth::captcha;
use crate::auth::captcha_webview;
use crate::superprops::{self, BROWSER_UA};

const LOGIN_URL: &str = "https://discord.com/api/v10/auth/login";
const LOGIN_PATH: &str = "/api/v10/auth/login";
const MFA_URL: &str = "https://discord.com/api/v10/auth/mfa/totp";
const MFA_PATH: &str = "/api/v10/auth/mfa/totp";

fn auth_http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("reqwest client")
}

/// Результат входа, сериализуется в IPC как есть.
#[derive(Debug)]
pub enum LoginOutcome {
    Token(String),
    Mfa { ticket: String, sms: bool },
}

/// Исход auth-POST c капча-обвязкой.
enum AuthPost {
    /// 2xx — тело ответа (текст JSON).
    Ok(String),
    /// Отказ без капчи: сырой JSON-текст для человекочитаемой ошибки.
    Rejected(String),
}

/// Общая обёртка «auth-POST с капча-обвязкой»: login и mfa могут упереться в
/// hCaptcha. Быстрый путь — reqwest (чистые контексты капчи не видят вовсе);
/// при 400+captcha весь запрос переезжает в окно discord.com
/// (captcha_webview::browser_post): страница сама делает POST, показывает
/// капчу с rqdata из СВОЕГО ответа и повторяет с заголовками X-Captcha-*
/// (≤ CAPTCHA_MAX_ATTEMPTS решений). `build_body` пересобирает тело на
/// каждую попытку (значения не Copy).
async fn post_with_captcha(
    app: &tauri::AppHandle,
    url: &str,
    path: &str,
    context: &str,
    build_body: &(dyn Fn() -> Value + Send + Sync),
) -> anyhow::Result<AuthPost> {
    let http = auth_http();
    let mut req = http
        .post(url)
        .header("Content-Type", "application/json")
        .header(
            "X-Super-Properties",
            superprops::super_properties(superprops::FALLBACK_BUILD_NUMBER),
        )
        .header("X-Discord-Locale", "ru-RU")
        .header("Accept-Language", "ru-RU,ru;q=0.9")
        .header("Origin", "https://discord.com");
    // Форма логина дополнительно помечает контекст (как веб-клиент).
    if context == "login" {
        req = req.header(
            "X-Context-Properties",
            superprops::context_properties_login(),
        );
    }
    let resp = req.json(&build_body()).send().await?;
    let status = resp.status();
    if status.is_success() {
        return Ok(AuthPost::Ok(resp.text().await.unwrap_or_default()));
    }
    let raw = resp.text().await.unwrap_or_default();
    // Капча — только на 400 с полем captcha_key:rqdata из reqwest-контекста
    // браузерный виджет не примет («Access Denied») — переносим запрос.
    if status.as_u16() == 400 {
        let body: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        if captcha::CaptchaChallenge::from_body(&body).is_some() {
            tracing::warn!("auth: {url} требует капчу — запрос уходит в окно discord.com");
            let ctx_props = if context == "login" {
                Some(superprops::context_properties_login())
            } else {
                None
            };
            // Живой client_build_number для X-Super-Properties повторного
            // запроса: статичный фолбэк устаревает, а несуществующий билд
            // в XSP поднимает риск-скоринг (лишние капчи).
            let build = superprops::fetch_build_number()
                .await
                .unwrap_or(superprops::FALLBACK_BUILD_NUMBER);
            let outcome = captcha_webview::browser_post(
                app,
                context,
                &[path],
                &build_body(),
                ctx_props.as_deref(),
                build,
            )
            .await?;
            return match outcome {
                captcha_webview::BrowserPost::Ok(text) => Ok(AuthPost::Ok(text)),
                captcha_webview::BrowserPost::Rejected(raw) => Ok(AuthPost::Rejected(raw)),
            };
        }
    }
    // Обычный отказ (ошибка формы / 429 / 5xx): тело — вызывающему.
    Ok(AuthPost::Rejected(raw))
}

/// Человекочитаемое сообщение из тела отказа: message → ошибки полей формы →
/// усечённый сырой JSON.
fn reject_message(raw: &str) -> String {
    let body: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    if let Some(msg) = body["message"].as_str() {
        return msg.to_string();
    }
    if let Some(fields) = body.as_object() {
        for (key, val) in fields {
            if let Some(msg) = val.as_str() {
                if matches!(key.as_str(), "password" | "login" | "email") {
                    return format!("Ошибка входа: {msg}");
                }
            }
        }
    }
    truncate(raw)
}

/// POST /auth/login (email+пароль; капча решается в окне discord.com).
pub async fn login(
    app: &tauri::AppHandle,
    email: &str,
    password: &str,
) -> anyhow::Result<LoginOutcome> {
    let body = || {
        json!({
            "login": email,          // поле называется `login`, не `email`!
            "password": password,
            "undelete": false,
            "login_source": null,
            "gift_code_sku_id": null
        })
    };
    match post_with_captcha(app, LOGIN_URL, LOGIN_PATH, "login", &body).await? {
        AuthPost::Ok(text) => {
            let outcome: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            outcome_from_response(outcome)
        }
        AuthPost::Rejected(raw) => anyhow::bail!("{}", reject_message(&raw)),
    }
}

/// Второй шаг 2FA: POST /auth/mfa/totp {code, ticket} → {token}.
pub async fn submit_mfa(
    app: &tauri::AppHandle,
    code: &str,
    ticket: &str,
) -> anyhow::Result<LoginOutcome> {
    let body = || json!({ "code": code, "ticket": ticket });
    match post_with_captcha(app, MFA_URL, MFA_PATH, "mfa", &body).await? {
        AuthPost::Ok(text) => {
            let outcome: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            outcome_from_response(outcome)
        }
        AuthPost::Rejected(raw) => anyhow::bail!("2FA: {}", reject_message(&raw)),
    }
}

/// Разбор успешного ответа auth-эндпоинта: {token} | {mfa, ticket}.
fn outcome_from_response(body: Value) -> anyhow::Result<LoginOutcome> {
    if let Some(token) = body["token"].as_str() {
        if !token.is_empty() {
            return Ok(LoginOutcome::Token(token.to_string()));
        }
    }
    // Успешный ответ без токена и с mfa:true — сервер требует 2FA.
    if body["mfa"].as_bool() == Some(true) {
        if let Some(ticket) = body["ticket"].as_str() {
            return Ok(LoginOutcome::Mfa {
                ticket: ticket.to_string(),
                sms: body["sms"].as_bool().unwrap_or(false),
            });
        }
    }
    anyhow::bail!(
        "Discord вернул успешный ответ без токена: {}",
        truncate(&body.to_string())
    )
}

fn truncate(s: &str) -> String {
    s.chars().take(200).collect()
}
