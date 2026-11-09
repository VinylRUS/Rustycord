//! CAPTCHA-инфраструктура — Discord может потребовать hCaptcha на любом
//! высокорисковом действии: вход email/паролем, обмен QR-тикета, инвайты,
//! заявки в друзья (docs.discord.food/topics/captcha-handling).
//!
//! Механика (спецификация):
//!  1. Эндпоинт отвечает 400 с полем `captcha_key` (+ sitekey, опционально
//!     `captcha_rqdata` — hCaptcha Enterprise, `captcha_session_id`,
//!     `captcha_rqtoken`, `should_serve_invisible`).
//!  2. Клиент показывает капчу пользователю. rqdata, если есть, ОБЯЗАНА
//!     уйти в виджет — но НЕ опцией render (api.js её молча игнорирует):
//!     только через `hcaptcha.setData(id, {rqdata})` на готовый виджет
//!     (так делает официальный клиент) — она попадает в getcaptcha и
//!     привязывает решение к вызову; без привязки siteverify отвечает
//!     invalid-response. Sitekey Discord привязан к домену, а rqdata
//!     Enterprise — к контексту сессии: и виджет, и POST, породивший
//!     вызов, живут в одном окне на discord.com
//!     (auth/captcha_webview.rs).
//!  3. Просьба повторяется с решением И В ЗАГОЛОВКАХ (`X-Captcha-Key` —
//!     решение, `X-Captcha-Session-Id`/`X-Captcha-Rqtoken` — привязка
//!     вызова), И В ПОЛЯХ JSON-ТЕЛА (`captcha_key`/`captcha_rqtoken`/
//!     `captcha_session_id`): веб-эндпоинты читают заголовки,
//!     remote-auth/login (путь десктопного QR-входа) — тело; второй
//!     механизм сервер игнорирует.
//!  4. Отказ (низкий trust score / повторное использование) — снова 400
//!     с новым вызовом: показываем следующий (не более CAPTCHA_MAX_ATTEMPTS).
//!
//! Отдельного `captcha_rqtoken` в вызове может не быть — тогда эхом в
//! ретрае идёт сам `captcha_rqdata` (в перехватах веб-клиента
//! X-Captcha-Rqtoken несёт именно значение rqdata из вызова; ретрай
//! без rqtoken отклоняется тем же 400).

use serde_json::Value;

/// Сколько капчей подряд показываем, прежде чем сдаться (каждый отказ
/// присылает новый вызов — зацикливаться нельзя).
pub const CAPTCHA_MAX_ATTEMPTS: u8 = 3;

/// Исход POST, выполненного целиком в браузерном контексте.
#[derive(Debug)]
pub enum BrowserPostOutcome {
    /// 2xx — тело ответа (текст JSON).
    Ok(String),
    /// Отказ: сырой JSON-текст для человекочитаемой ошибки.
    Rejected(String),
}

/// Решатель капчи в браузерном контексте. Контракт — «весь POST уезжает в
/// окно discord.com»: страница сама делает запрос, показывает виджет с
/// rqdata из СВОЕГО ответа и повторяет с заголовками X-Captcha-*
/// (≤ [`CAPTCHA_MAX_ATTEMPTS`] решений). Реализации:
///  * Tauri — `src-tauri/src/captcha_webview.rs` (WebviewWindow);
///  * GPUI — `app-gpui/src/captcha.rs` (standalone wry).
///
/// `context` — место вызова ("login" | "mfa" | "qr"), `paths` — API-пути
/// для повтора (первый рабочий выигрывает), `body` — JSON-тело, `ctx_props`
/// — X-Context-Properties (форма логина), `build` — свежий
/// client_build_number для X-Super-Properties.
#[async_trait::async_trait]
pub trait CaptchaSolver: Send + Sync + 'static {
    async fn browser_post(
        &self,
        context: &str,
        paths: &[&str],
        body: &Value,
        ctx_props: Option<&str>,
        build: u64,
    ) -> anyhow::Result<BrowserPostOutcome>;
}

/// Один вызов hCaptcha, разобранный из тела ответа Discord.
#[derive(Debug, Clone)]
pub struct CaptchaChallenge {
    pub sitekey: String,
    /// hCaptcha Enterprise: ОБЯЗАН уйти в виджет (иначе решение не примут).
    pub rqdata: Option<String>,
    /// Токен вызова для ретрая (заголовок X-Captcha-Rqtoken + поле тела
    /// captcha_rqtoken): явный `captcha_rqtoken` из ответа, а при его
    /// отсутствии — эхо `captcha_rqdata`.
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
                // Без rqtoken повторный POST отклоняется тем же 400
                // (цикл одинаковых отказов): если отдельного поля нет,
                // эхом идёт сам rqdata — так делает официальный веб-клиент.
                .or_else(|| body.get("captcha_rqdata").and_then(|v| v.as_str()))
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
