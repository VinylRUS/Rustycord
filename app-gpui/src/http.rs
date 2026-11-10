//! HTTP-клиент для GPUI (аватарки/эмодзи через `img()`): реализация
//! трейта `gpui::http_client::HttpClient` поверх reqwest с rustls.
//!
//! gpui рендерит удалённые картинки только при зарегистрированном
//! глобальном клиенте (`cx.set_http_client`) — отдаём ему наш reqwest.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::AsyncReadExt;
use gpui::http_client::http::{header, HeaderValue, Request, Response, StatusCode};
use gpui::http_client::{AsyncBody, HttpClient, Url};

pub struct ReqwestHttpClient {
    client: reqwest::Client,
    user_agent: HeaderValue,
}

impl ReqwestHttpClient {
    pub fn new() -> anyhow::Result<Self> {
        let user_agent =
            HeaderValue::from_str(&format!("RustyCord-GPUI/{}", rustycord_core::VERSION))
                .unwrap_or_else(|_| HeaderValue::from_static("RustyCord-GPUI"));
        let client = reqwest::Client::builder()
            .user_agent(user_agent.to_str().unwrap_or("RustyCord-GPUI"))
            .build()?;
        Ok(Self { client, user_agent })
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new().expect("reqwest client")
    }
}

/// Прочитать AsyncBody в байты (тела картинок маленькие — буферизуем целиком).
async fn body_to_vec(mut body: AsyncBody) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    body.read_to_end(&mut out)
        .await
        .map_err(|e| anyhow::anyhow!("read body: {e}"))?;
    Ok(out)
}

impl HttpClient for ReqwestHttpClient {
    fn type_name(&self) -> &'static str {
        "rustycord-reqwest"
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        // Системный прокси — на совести reqwest (читает env).
        None
    }

    fn send(
        &self,
        req: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let client = self.client.clone();
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let bytes = body_to_vec(body).await?;

            let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?;
            let url: reqwest::Url = parts
                .uri
                .to_string()
                .parse()
                .map_err(|e| anyhow::anyhow!("url: {e}"))?;

            let mut request = client.request(method, url);
            for (name, value) in parts.headers.iter() {
                // reqwest сам ведёт host/content-length.
                if name == header::HOST || name == header::CONTENT_LENGTH {
                    continue;
                }
                request = request.header(name, value);
            }

            // reqwest требует Tokio-контекст (DNS-резолвер hyper-util уходит в
            // spawn_blocking), а GPUI поллит этот future своим executor'ом —
            // «there is no reactor running» (v0.11.0 падало на первой аватарке).
            // Выполняем запрос на протокольном рантайме и возвращаем результат
            // через oneshot: await Receiver вне Tokio безопасен.
            let (tx, rx) = tokio::sync::oneshot::channel();
            crate::bridge::runtime().spawn(async move {
                let result = async {
                    let response = request.body(bytes).send().await?;
                    let status = StatusCode::from_u16(response.status().as_u16())?;
                    let mut builder = Response::builder().status(status);
                    for (name, value) in response.headers().iter() {
                        builder = builder.header(name, value);
                    }
                    let body = response.bytes().await?.to_vec();
                    builder
                        .body(AsyncBody::from(body))
                        .map_err(|e| anyhow::anyhow!("build response: {e}"))
                }
                .await;
                // Получатель мог уйти (GPUI отменил загрузку) — тихо игнорируем.
                let _ = tx.send(result);
            });
            rx.await
                .map_err(|e| anyhow::anyhow!("http task died: {e}"))?
        })
    }
}

pub fn shared() -> Arc<dyn HttpClient> {
    Arc::new(ReqwestHttpClient::new().expect("reqwest client"))
}
