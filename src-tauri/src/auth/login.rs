//! POST /auth/login — вход email+паролем, ровно как форма на discord.com/login.
//!
//! Исходы, как у оригинала:
//!  * `{token}` — готово;
//!  * `{mfa: true, ticket}` — нужен код 2FA (TOTP), добивается вторым запросом
//!    POST /auth/mfa/totp;
//!  * 400 с полем `captcha_key` — Discord требует hCaptcha: показываем
//!    пользователю модалку (auth/captcha.rs) и повторяем запрос с
//!    заголовками X-Captcha-Key/-Session-Id/-Rqtoken
//!    (docs.discord.food/topics/captcha-handling).

use serde_json::{json, Value};

use crate::auth::captcha::{self, CaptchaBroker, SolvedCaptcha};
use crate::superprops::{self, BROWSER_UA};

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
    /// 2xx — разбирай тело ({token} | {mfa, ticket}).
    Ok(reqwest::Response),
    /// Отказ без капчи: сырой JSON-текст для человекочитаемой ошибки.
    Rejected(String),
}

/// Общая обёртка «auth-POST с капча-ретраем»: login и mfa могут упереться в
/// hCaptcha — решение прилетает из UI-модалки (broker) и запрос повторяется
/// заголовками X-Captcha-* (до CAPTCHA_MAX_ATTEMPTS раз). `build_body`
/// пересобирает тело на каждую попытку (значения не Copy).
async fn post_with_captcha(
    app: &tauri::AppHandle,
    broker: &CaptchaBroker,
    url: &str,
    context: &str,
    build_body: &(dyn Fn() -> Value + Send + Sync),
) -> anyhow::Result<AuthPost> {
    let http = auth_http();
    let mut solved: Option<SolvedCaptcha> = None;
    for attempt in 0..captcha::CAPTCHA_MAX_ATTEMPTS {
        let req = http
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
        let mut req = if context == "login" {
            req.header(
                "X-Context-Properties",
                superprops::context_properties_login(),
            )
        } else {
            req
        }
        .json(&build_body());
        if let Some(s) = &solved {
            req = s.apply(req);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(AuthPost::Ok(resp));
        }
        let raw = resp.text().await.unwrap_or_default();
        // Капча — только на 400 с полем captcha_key: решаем и повторяем.
        if status.as_u16() == 400 {
            let body: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            if let Some(challenge) = captcha::CaptchaChallenge::from_body(&body) {
                if attempt + 1 >= captcha::CAPTCHA_MAX_ATTEMPTS {
                    anyhow::bail!(
                        "капча: Discord отклонил решение {} раз подряд — попробуй позже",
                        captcha::CAPTCHA_MAX_ATTEMPTS
                    );
                }
                tracing::warn!(
                    "auth: {url} требует капчу (попытка {}) — показываем пользователю",
                    attempt + 1
                );
                let solution = broker.present(app, context, &challenge).await?;
                solved = Some(SolvedCaptcha::from_solution(solution, &challenge)?);
                continue;
            }
        }
        // Обычный отказ (ошибка формы / 429 / 5xx): тело — вызывающему.
        return Ok(AuthPost::Rejected(raw));
    }
    unreachable!("капча-цикл выходит через return/bail")
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

/// POST /auth/login (email+пароль; капча решается модалкой при требовании).
pub async fn login(
    app: &tauri::AppHandle,
    broker: &CaptchaBroker,
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
    match post_with_captcha(
        app,
        broker,
        "https://discord.com/api/v10/auth/login",
        "login",
        &body,
    )
    .await?
    {
        AuthPost::Ok(resp) => {
            let outcome: Value = resp.json().await.unwrap_or(Value::Null);
            outcome_from_response(outcome)
        }
        AuthPost::Rejected(raw) => anyhow::bail!("{}", reject_message(&raw)),
    }
}

/// Второй шаг 2FA: POST /auth/mfa/totp {code, ticket} → {token}.
pub async fn submit_mfa(
    app: &tauri::AppHandle,
    broker: &CaptchaBroker,
    code: &str,
    ticket: &str,
) -> anyhow::Result<LoginOutcome> {
    let body = || json!({ "code": code, "ticket": ticket });
    match post_with_captcha(
        app,
        broker,
        "https://discord.com/api/v10/auth/mfa/totp",
        "mfa",
        &body,
    )
    .await?
    {
        AuthPost::Ok(resp) => {
            let outcome: Value = resp.json().await.unwrap_or(Value::Null);
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
