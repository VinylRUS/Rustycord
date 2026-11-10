//! Shared WebSocket plumbing for the gateway and voice connections.
//!
//! `connect_ws` — быстрый способ открыть клиентский WebSocket: DNS с
//! таймаутом → гонка TCP-подключений по всем адресам хоста → TLS и
//! WS-handshake. Замена «голой» `tokio_tungstenite::connect_async`,
//! которая перебирает адреса ПОСЛЕДОВАТЕЛЬНО и без таймаута: на
//! деградированных сетях (DPI-дросселирование Discord в отдельных
//! странах) первый «мёртвый» IP выжигает ~20 секунд SYN-ретрансмитов
//! на каждый адрес — отсюда «QR-код ждём 40 секунд» и «вечное
//! подключение» после логина.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub type WsWriter = SplitSink<WsStream, tokio_tungstenite::tungstenite::Message>;
pub type WsReader = SplitStream<WsStream>;

/// Таймаут DNS-резолва: мёртвый системный резолвер не должен вешать экран
/// логина (getaddrinfo без таймаута молчит вечно).
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
/// Общий бюджет подключения: DNS + гонка TCP + TLS + WS-handshake.
/// Меньше таймаута REST-клиента (25 с): WS-эндпоинты Discord лёгкие,
/// 15 с хватает с запасом даже на плохой сети.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Открыть клиентский WebSocket по запросу (wss://…).
///
/// Порядок: DNS с таймаутом → параллельная гонка TCP по всем адресам
/// (первый ответивший побеждает, проигравшие отменяются дропом) → TLS +
/// handshake тем же дефолтным rustls-коннектором с webpki-корнями, что
/// использует `connect_async` (поведение бинарно совпадает, меняется
/// только скорость установления).
pub async fn connect_ws<R>(request: R) -> Result<WsStream>
where
    R: IntoClientRequest + Unpin,
{
    let request = request.into_client_request()?;
    tokio::time::timeout(CONNECT_TIMEOUT, connect_ws_inner(request))
        .await
        .map_err(|_| {
            anyhow!(
                "не удалось подключиться за {} с — сеть до Discord не отвечает \
                 (проверь интернет/провайдера)",
                CONNECT_TIMEOUT.as_secs()
            )
        })?
}

async fn connect_ws_inner(request: Request) -> Result<WsStream> {
    let domain = request
        .uri()
        .host()
        .ok_or_else(|| anyhow!("в URL WebSocket нет хоста"))?
        .to_string();
    let port = request
        .uri()
        .port_u16()
        .unwrap_or(match request.uri().scheme_str() {
            Some("ws") => 80,
            _ => 443,
        });

    // 1. DNS с таймаутом.
    let lookup = tokio::time::timeout(
        RESOLVE_TIMEOUT,
        tokio::net::lookup_host((domain.as_str(), port)),
    )
    .await
    .map_err(|_| anyhow!("DNS не ответил за {} с", RESOLVE_TIMEOUT.as_secs()))?;
    let addrs: Vec<SocketAddr> = lookup
        .with_context(|| format!("DNS: хост {domain} не разрешается"))?
        .collect();
    if addrs.is_empty() {
        bail!("DNS: {domain} разрешился в пустой список адресов");
    }
    tracing::debug!(
        "ws-connect: {domain} → {} адрес(а), стартуем гонку подключений",
        addrs.len()
    );

    // 2. Гонка TCP-подключений.
    let stream = race_connect(addrs).await?;

    // 3. TLS + WS-handshake поверх уже подключённого стрима: тот же
    //    дефолтный коннектор (rustls + webpki-корни), что внутри
    //    connect_async — SNI, ALPN и проверка сертификатов без изменений.
    let ws = tokio_tungstenite::client_async_tls_with_config(request, stream, None, None)
        .await
        .context("TLS/WS-handshake")?
        .0;
    Ok(ws)
}

/// Отдельная попытка TCP-подключения (бокснутая, чтобы жить в Vec и
/// участвовать в select_all).
type ConnectFut =
    std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<TcpStream>> + Send>>;

/// Параллельное TCP-подключение: `select_all` по всем адресам, ошибки
/// выбывают из гонки, первый успешный сокет побеждает; остальные попытки
/// отменяются дропом фьючерса.
async fn race_connect(addrs: Vec<SocketAddr>) -> Result<TcpStream> {
    let mut futures: Vec<ConnectFut> = addrs
        .into_iter()
        .map(|addr| {
            let fut: ConnectFut = Box::pin(async move { TcpStream::connect(addr).await });
            fut
        })
        .collect();
    let mut last_err: Option<String> = None;
    while !futures.is_empty() {
        let (result, _idx, rest) = futures_util::future::select_all(futures).await;
        futures = rest;
        match result {
            Ok(stream) => {
                // TCP_NODELAY — интерактивный протокол: задержка 200 мс на
                // маленьких кадрах (heartbeat/typing) не нужна.
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(e) => {
                tracing::debug!("ws-connect: адрес не ответил: {e}");
                last_err = Some(e.to_string());
            }
        }
    }
    bail!(
        "ни один адрес не ответил: {}",
        last_err.unwrap_or_else(|| "нет адресов".to_string())
    )
}

/// Read the next JSON text frame, skipping non-text frames.
pub async fn next_json(reader: &mut WsReader) -> Result<Value> {
    loop {
        match reader.next().await {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                return Ok(serde_json::from_str(&text)?);
            }
            Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(_)))
            | Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(_))) => continue,
            Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
                bail!("websocket закрыт: {frame:?}");
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(e.into()),
            None => bail!("websocket поток завершился"),
        }
    }
}

/// Send a JSON frame as text.
pub async fn send_json(writer: &mut WsWriter, value: &Value) -> Result<()> {
    writer
        .send(tokio_tungstenite::tungstenite::Message::Text(
            value.to_string(),
        ))
        .await?;
    Ok(())
}

/// Split a freshly connected stream into (writer, reader).
pub fn split(ws: WsStream) -> (WsWriter, WsReader) {
    ws.split()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_ws_rejects_bad_url() {
        // Битый URL ловится на этапе into_client_request/DNS — без сети
        // и без зависания (tungstenite трактует «not-a-url» как хост ws://).
        let started = std::time::Instant::now();
        let err = connect_ws("not-a-url").await.unwrap_err();
        assert!(started.elapsed() < CONNECT_TIMEOUT);
        assert!(
            !err.to_string().is_empty(),
            "ожидали содержательную ошибку, получили: {err:#}"
        );
    }

    #[tokio::test]
    async fn connect_ws_fails_fast_on_dead_dns() {
        // .invalid (RFC 6761) не разрешается никаким резолвером: функция
        // обязана быстро вернуться с ошибкой DNS, а не зависнуть. Даже если
        // системный DNS молчит, выручает RESOLVE_TIMEOUT.
        let started = std::time::Instant::now();
        let err = connect_ws("wss://must-fail.invalid").await.unwrap_err();
        assert!(started.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(2));
        assert!(!err.to_string().is_empty(), "ожидали содержательную ошибку");
    }
}
