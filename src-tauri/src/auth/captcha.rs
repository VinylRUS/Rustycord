//! CAPTCHA-инфраструктура — Discord может потребовать hCaptcha на любом
//! высокорисковом действии: вход email/паролем, обмен QR-тикета, инвайты,
//! заявки в друзья (docs.discord.food/topics/captcha-handling).
//!
//! Механика (спецификация):
//!  1. Эндпоинт отвечает 400 с полем `captcha_key` (+ sitekey, опционально
//!     `captcha_rqdata` — hCaptcha Enterprise, `captcha_session_id`,
//!     `captcha_rqtoken`, `should_serve_invisible`).
//!  2. Клиент показывает капчу пользователю. rqdata, если есть, ОБЯЗАНА
//!     уйти в виджет (иначе решение не примут).
//!  3. Просьба повторяется с заголовками:
//!     `X-Captcha-Key` — решение, `X-Captcha-Session-Id` и
//!     `X-Captcha-Rqtoken` — если были в вызове.
//!  4. Отказ (низкий trust score / повторное использование) — снова 400
//!     с новым вызовом: показываем следующий (не более CAPTCHA_MAX_ATTEMPTS).
//!
//! Перенос решения в JSON-теле (`captcha_key` в body) задепрекейтен —
//! только заголовки.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::Value;
use tauri::Emitter;
use tokio::sync::oneshot;

/// Сколько капчей подряд показываем, прежде чем сдаться (каждый отказ
/// присылает новый вызов — зацикливаться нельзя).
pub const CAPTCHA_MAX_ATTEMPTS: u8 = 3;

/// Как долго ждём решения пользователя (QR-тикет живёт минуты —
/// щедрый таймаут, но не бесконечный).
const CAPTCHA_TIMEOUT: Duration = Duration::from_secs(300);

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

/// Ответ пользователя на капчу.
#[derive(Debug, Clone)]
pub enum CaptchaSolution {
    /// Токен hCaptcha (h-captcha-response).
    Key(String),
    /// Пользователь закрыл окно / ушёл с экрана.
    Cancelled,
}

/// Решённая капча: токен + метаданные вызова, которые Discord ждёт
/// в заголовках ретрая (X-Captcha-Key / -Session-Id / -Rqtoken).
#[derive(Debug, Clone)]
pub struct SolvedCaptcha {
    pub key: String,
    pub session_id: Option<String>,
    pub rqtoken: Option<String>,
}

impl SolvedCaptcha {
    /// Собрать из решения пользователя + вызова, на который оно отвечено.
    pub fn from_solution(solution: CaptchaSolution, challenge: &CaptchaChallenge) -> Result<Self> {
        match solution {
            CaptchaSolution::Key(key) => Ok(Self {
                key,
                session_id: challenge.session_id.clone(),
                rqtoken: challenge.rqtoken.clone(),
            }),
            CaptchaSolution::Cancelled => bail!("капча отменена пользователем"),
        }
    }

    /// Наложить заголовки ретрая на запрос (docs: решение — ТОЛЬКО
    /// заголовками, JSON-поле captcha_key задепрекейчено).
    pub fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req.header("X-Captcha-Key", &self.key);
        let req = match &self.session_id {
            Some(sid) => req.header("X-Captcha-Session-Id", sid),
            None => req,
        };
        match &self.rqtoken {
            Some(rqt) => req.header("X-Captcha-Rqtoken", rqt),
            None => req,
        }
    }
}

/// Мост «Rust-запрос ↔ UI-модалка»: запрос, упёршийся в капчу, регистрирует
/// себе канал ответа (`present`), UI решает капчу и возвращает токен
/// (`submit`/`cancel` через Tauri-команды). Клонируется: живёт в AppState.
#[derive(Clone, Default)]
pub struct CaptchaBroker {
    inner: Arc<Mutex<Option<oneshot::Sender<CaptchaSolution>>>>,
}

impl CaptchaBroker {
    /// Показать пользователю вызов `context`-а («qr» | «login» | …) и ждать
    /// решения. Блокирует запрос до решения/отмены/таймаута — так и надо:
    /// без капчи Discord этот запрос всё равно не примет.
    pub async fn present(
        &self,
        app: &tauri::AppHandle,
        context: &str,
        challenge: &CaptchaChallenge,
    ) -> Result<CaptchaSolution> {
        let (tx, rx) = oneshot::channel();
        {
            let mut slot = self.inner.lock().unwrap();
            // Прежний вайтер (не должно быть) дропается — его present
            // увидит закрытый канал и честно вернёт ошибку.
            *slot = Some(tx);
        }
        let _ = app.emit(
            "discord://captcha",
            serde_json::json!({
                "stage": "challenge",
                "context": context,
                "sitekey": challenge.sitekey,
                "rqdata": challenge.rqdata,
                "rqtoken": challenge.rqtoken,
                "sessionId": challenge.session_id,
                "invisible": challenge.invisible,
            }),
        );
        match tokio::time::timeout(CAPTCHA_TIMEOUT, rx).await {
            Ok(Ok(solution)) => Ok(solution),
            Ok(Err(_dropped)) => bail!("капча: окно закрыто без решения"),
            Err(_) => {
                self.inner.lock().unwrap().take();
                bail!("капча: время на решение вышло (5 минут)")
            }
        }
    }

    /// UI прислал решение.
    pub fn submit(&self, key: String) -> bool {
        if let Some(tx) = self.inner.lock().unwrap().take() {
            tx.send(CaptchaSolution::Key(key)).is_ok()
        } else {
            false
        }
    }

    /// UI закрыл модалку без решения.
    pub fn cancel(&self) -> bool {
        if let Some(tx) = self.inner.lock().unwrap().take() {
            tx.send(CaptchaSolution::Cancelled).is_ok()
        } else {
            false
        }
    }
}
