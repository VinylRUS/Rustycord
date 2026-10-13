//! POST /auth/login — вход email+паролем, ровно как форма на discord.com/login.
//!
//! Три исхода, как у оригинала:
//!  * `{token}` — готово;
//!  * `{mfa: true, ticket}` — нужен код 2FA (TOTP), добивается вторым запросом
//!    POST /auth/mfa/totp;
//!  * 400 с полем `captcha_key` — Discord требует капчу (подозрительный
//!    IP/новый логин) — капчу через API не пройти, честно отправляем
//!    пользователя на QR-код.

use serde_json::{json, Value};

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

/// POST /auth/login (или /auth/mfa/totp, если передан ticket + code).
pub async fn login(email: &str, password: &str) -> anyhow::Result<LoginOutcome> {
    let http = auth_http();
    let resp = http
        .post("https://discord.com/api/v10/auth/login")
        .header("Content-Type", "application/json")
        .header(
            "X-Super-Properties",
            superprops::super_properties(superprops::FALLBACK_BUILD_NUMBER),
        )
        .header(
            "X-Context-Properties",
            superprops::context_properties_login(),
        )
        .header("X-Discord-Locale", "ru-RU")
        .header("Accept-Language", "ru-RU,ru;q=0.9")
        .header("Origin", "https://discord.com")
        .json(&json!({
            "login": email,          // поле называется `login`, не `email`!
            "password": password,
            "undelete": false,
            "login_source": null,
            "gift_code_sku_id": null
        }))
        .send()
        .await?;

    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);

    if status.is_success() {
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
        );
    }

    // 400 + captcha_key → сайт хочет hcaptcha, API-путём это не обходится.
    if body.get("captcha_key").is_some() {
        anyhow::bail!(
            "Discord требует капчу для этого входа (подозрительная сеть или новый логин). \
             Войди через QR-код — он капчу не требует."
        );
    }
    // Обычная ошибка формы: {"password": "Неверный пароль"} или {"login": "..."}.
    if let Some(fields) = body.as_object() {
        for (key, val) in fields {
            if let Some(msg) = val.as_str() {
                if matches!(key.as_str(), "password" | "login" | "email") {
                    anyhow::bail!("Ошибка входа: {msg}");
                }
            }
        }
    }
    anyhow::bail!(
        "Discord API {}: {}",
        status.as_u16(),
        truncate(&body.to_string())
    )
}

/// Второй шаг 2FA: POST /auth/mfa/totp {code, ticket} → {token}.
pub async fn submit_mfa(code: &str, ticket: &str) -> anyhow::Result<LoginOutcome> {
    let http = auth_http();
    let resp = http
        .post("https://discord.com/api/v10/auth/mfa/totp")
        .header("Content-Type", "application/json")
        .header(
            "X-Super-Properties",
            superprops::super_properties(superprops::FALLBACK_BUILD_NUMBER),
        )
        .header("X-Discord-Locale", "ru-RU")
        .header("Origin", "https://discord.com")
        .json(&json!({ "code": code, "ticket": ticket }))
        .send()
        .await?;

    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);

    if status.is_success() {
        if let Some(token) = body["token"].as_str() {
            if !token.is_empty() {
                return Ok(LoginOutcome::Token(token.to_string()));
            }
        }
        anyhow::bail!("MFA-ответ без токена: {}", truncate(&body.to_string()));
    }
    if let Some(msg) = body["message"].as_str() {
        anyhow::bail!("2FA отклонено: {msg}");
    }
    anyhow::bail!(
        "Discord API {}: {}",
        status.as_u16(),
        truncate(&body.to_string())
    )
}

fn truncate(s: &str) -> String {
    s.chars().take(200).collect()
}
