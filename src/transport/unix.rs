#[cfg(unix)]
use async_trait::async_trait;
#[cfg(unix)]
use futures_util::{SinkExt, StreamExt};
#[cfg(unix)]
use http::Request;
#[cfg(unix)]
use serde_json::Value;
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(unix)]
use tokio_tungstenite::{client_async, tungstenite::protocol::Message, WebSocketStream};

#[cfg(unix)]
use super::AppServerTransport;

#[cfg(unix)]
pub struct UnixTransport {
    stream: WebSocketStream<UnixStream>,
}

#[cfg(unix)]
impl UnixTransport {
    pub async fn connect(path: &std::path::Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let request = Request::builder()
            .uri("ws://localhost/")
            .header("Host", "localhost")
            .body(())?;
        let (stream, _) = client_async(request, stream).await?;
        Ok(Self { stream })
    }
}

#[cfg(unix)]
#[async_trait]
impl AppServerTransport for UnixTransport {
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        self.stream
            .send(Message::Text(message.to_string().into()))
            .await?;
        Ok(())
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        while let Some(message) = self.stream.next().await {
            match message? {
                Message::Text(text) => return Ok(Some(serde_json::from_str(&text)?)),
                Message::Binary(bytes) => return Ok(Some(serde_json::from_slice(&bytes)?)),
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok(None)
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        let _ = self.stream.close(None).await;
        Ok(())
    }
}
