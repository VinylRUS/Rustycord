//! Rate-limit-aware REST client for the Discord API.
//!
//! Reverse-engineered behaviour we implement:
//!  * every response carries `X-RateLimit-Bucket / Remaining / Reset-After`,
//!    so the client can pre-emptively sleep before the bucket resets;
//!  * buckets key on the *route* (channels/{id}/messages), not the endpoint,
//!    and only the first path segment after a "major" resource matters;
//!  * 429 responses carry `retry_after` (seconds, may be fractional) and can
//!    be global (`X-RateLimit-Global: true`) — global limits pause everything.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::{Client, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const BASE_URL: &str = "https://discord.com/api/v10";

#[derive(Clone, Copy, Debug)]
struct Bucket {
    remaining: i64,
    reset_at: Instant,
}

pub struct RestClient {
    http: Client,
    /// bucket id → state
    buckets: Mutex<HashMap<String, Bucket>>,
    /// route key → bucket id (learned from response headers)
    routes: Mutex<HashMap<String, String>>,
}

/// Conservative route key: keeps major params (channels/{id}, guilds/{id}),
/// masks every other snowflake as `{id}`.
fn route_key(method: &Method, path: &str) -> String {
    let segs: Vec<String> = path
        .trim_matches('/')
        .split('/')
        .map(String::from)
        .collect();
    let mut out: Vec<String> = Vec::with_capacity(segs.len());
    for (i, seg) in segs.iter().enumerate() {
        let is_major = i == 1
            && matches!(
                segs.first().map(String::as_str),
                Some("channels") | Some("guilds") | Some("webhooks")
            );
        if i > 0 && !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()) && !is_major {
            out.push("{id}".to_string());
        } else {
            out.push(seg.clone());
        }
    }
    format!("{}:{}", method, out.join("/"))
}

/// Minimal percent-encoder for path segments (emoji in reaction routes).
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

impl RestClient {
    /// Клиент для **пользовательской** сессии: Authorization без префикса
    /// `Bot`, плюс «паспорт браузера» — X-Super-Properties и браузерный UA.
    /// Без них юзер-токены получают 401 на большинстве write-эндпоинтов.
    pub fn new(token: &str, build_number: u64) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        let auth = reqwest::header::HeaderValue::from_str(token)
            .map_err(|_| anyhow::anyhow!("Токен содержит недопустимые символы"))?;
        headers.insert(reqwest::header::AUTHORIZATION, auth);
        let sp = reqwest::header::HeaderValue::from_str(&crate::superprops::super_properties(
            build_number,
        ))
        .map_err(|_| anyhow::anyhow!("super-properties: недопустимые символы"))?;
        headers.insert("X-Super-Properties", sp);
        headers.insert(
            "X-Discord-Locale",
            reqwest::header::HeaderValue::from_static("ru-RU"),
        );
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("ru-RU,ru;q=0.9"),
        );
        let http = Client::builder()
            .default_headers(headers)
            // Маскируемся под браузерный клиент — то же, что делает Vesktop:
            // серверы Discord видят обычную сессию discord.com/app.
            .user_agent(crate::superprops::BROWSER_UA)
            .timeout(Duration::from_secs(25))
            .build()?;
        Ok(Self {
            http,
            buckets: Mutex::new(HashMap::new()),
            routes: Mutex::new(HashMap::new()),
        })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request(Method::GET, path, &[], None).await
    }

    pub async fn get_json_q<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<T> {
        self.request(Method::GET, path, query, None).await
    }

    pub async fn post_json<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        self.request(Method::POST, path, &[], Some(body)).await
    }

    pub async fn patch_json<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        self.request(Method::PATCH, path, &[], Some(body)).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request(Method::DELETE, path, &[], None).await
    }

    /// PUT without a body (e.g. adding a reaction, creating a ban).
    pub async fn put_empty(&self, path: &str) -> Result<()> {
        self.request(Method::PUT, path, &[], None).await
    }

    /// PUT with a JSON body (e.g. ban with delete_message_seconds).
    pub async fn put_json<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        self.request(Method::PUT, path, &[], Some(body)).await
    }

    /// POST without a body (e.g. the typing trigger).
    pub async fn post_empty(&self, path: &str) -> Result<()> {
        self.request(Method::POST, path, &[], None).await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Value>,
    ) -> Result<T> {
        let key = route_key(&method, path);
        for _ in 0..6 {
            if let Some(wait) = self.wait_for(&key) {
                tracing::debug!(path, ?wait, "rate limit: waiting for bucket reset");
                tokio::time::sleep(wait).await;
            }

            let url = format!("{}{}", BASE_URL, path);
            let mut req = self.http.request(method.clone(), &url);
            if !query.is_empty() {
                req = req.query(query);
            }
            if let Some(b) = &body {
                req = req.json(b);
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("запрос {method} {path}"))?;

            // --- rate limit bookkeeping from response headers ---
            let hdr = |name: &str| -> Option<String> {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
            };
            if let Some(bucket_id) = hdr("x-ratelimit-bucket") {
                self.routes
                    .lock()
                    .unwrap()
                    .insert(key.clone(), bucket_id.clone());
                let remaining: i64 = hdr("x-ratelimit-remaining")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1);
                let reset_after: f64 = hdr("x-ratelimit-reset-after")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
                let map_key = if hdr("x-ratelimit-global").as_deref() == Some("true") {
                    "global".to_string()
                } else {
                    bucket_id
                };
                self.buckets.lock().unwrap().insert(
                    map_key,
                    Bucket {
                        remaining,
                        reset_at: Instant::now()
                            + Duration::from_secs_f64(reset_after.max(0.0) + 0.05),
                    },
                );
            }

            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS {
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                let retry_after: f64 = body
                    .get("retry_after")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                tracing::warn!(path, retry_after, "429 rate limited — backing off");
                tokio::time::sleep(Duration::from_secs_f64(retry_after.max(0.25))).await;
                continue;
            }
            if status == StatusCode::NO_CONTENT {
                // 204 No Content — e.g. typing trigger, message delete
                return Ok(serde_json::from_value(Value::Null)?);
            }
            let text = resp.text().await?;
            if !status.is_success() {
                let msg = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
                    .unwrap_or_else(|| text.chars().take(200).collect());
                bail!("Discord API {} {}: {}", status.as_u16(), path, msg);
            }
            return Ok(serde_json::from_str(&text)?);
        }
        bail!("превышено число повторов после rate limit: {path}")
    }

    fn wait_for(&self, route: &str) -> Option<Duration> {
        let bucket_id = self.routes.lock().unwrap().get(route).cloned()?;
        let buckets = self.buckets.lock().unwrap();
        let b = buckets.get(&bucket_id)?;
        if b.remaining <= 0 && b.reset_at > Instant::now() {
            Some(b.reset_at - Instant::now())
        } else {
            None
        }
    }
}
