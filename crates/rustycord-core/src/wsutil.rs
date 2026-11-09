//! Shared WebSocket plumbing for the gateway and voice connections.

use anyhow::{bail, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub type WsWriter = SplitSink<WsStream, tokio_tungstenite::tungstenite::Message>;
pub type WsReader = SplitStream<WsStream>;

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
