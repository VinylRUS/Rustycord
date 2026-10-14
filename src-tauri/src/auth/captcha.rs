//! CAPTCHA-инфраструктура — Discord может потребовать hCaptcha на любом
//! высокорисковом действии: вход email/паролем, обмен QR-тикета, инвайты,
//! заявки в друзья (docs.discord.food/topics/captcha-handling).
//!
//! Механика (спецификация):
//!  1. Эндпоинт отвечает 400 с полем `captcha_key` (+ sitekey, опционально
//!     `captcha_rqdata` — hCaptcha Enterprise, `captcha_session_id`,
//!     `captcha_rqtoken`, `should_serve_invisible`).
//!  2. Клиент показывает капчу пользователю. rqdata, если есть, ОБЯЗАНА
//!     уйти в виджет (иначе решение не примут). Sitekey Discord привязан
//!     к домену, а rqdata Enterprise — к контексту сессии: и виджет, и
//!     POST, породивший вызов, живут в одном окне на discord.com
//!     (auth/captcha_webview.rs).
//!  3. Просьба повторяется заголовками: `X-Captcha-Key` — решение,
//!     `X-Captcha-Session-Id` и `X-Captcha-Rqtoken` — если были в вызове.
//!  4. Отказ (низкий trust score / повторное использование) — снова 400
//!     с новым вызовом: показываем следующий (не более CAPTCHA_MAX_ATTEMPTS).
//!
//! Перенос решения в JSON-теле (`captcha_key` в body) задепрекейтен —
//! только заголовки.

use serde_json::Value;

/// Сколько капчей подряд показываем, прежде чем сдаться (каждый отказ
/// присылает новый вызов — зацикливаться нельзя).
pub const CAPTCHA_MAX_ATTEMPTS: u8 = 3;

/// Один вызов hCaptcha, разобранный из тела ответа Discord.
#[derive(Debug, Clone)]
pub struct CaptchaChallenge {
    pub sitekey: String,
    /// hCaptcha Enterprise: ОБЯЗАН уйти в виджет (иначе решение не примут).
    pub rqdata: Option<String>,
    /// Токен запроса вызова — в ретрай как X-Captcha-Rqtoken.
    pub rqtoken: Option<String>,
    /// Привязка решения к сессии — в ретрай как X-Captcha-Session-Id.
    pub session_id: Option<String>,
    /// Невидимая капча (проходит без кликов).
    pub invisible: bool,
}

impl CaptchaChallenge {
    /// 400 + `captcha_key` в теле → вызов. Любой другой ответ → None.
    pub fn from_body(body: &Value) -> Option<Self> {
        body.get("captcha_key")?;
        let sitekey = body
            .get("captcha_sitekey")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if sitekey.is_empty() {
            // Старые эндпоинты иногда дают только сервис: рисовать нечего.
            return None;
        }
        Some(Self {
            sitekey,
            rqdata: body
                .get("captcha_rqdata")
                .and_then(|v| v.as_str())
                .map(String::from),
            rqtoken: body
                .get("captcha_rqtoken")
                .and_then(|v| v.as_str())
                .map(String::from),
            session_id: body
                .get("captcha_session_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            invisible: body
                .get("should_serve_invisible")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }
}
