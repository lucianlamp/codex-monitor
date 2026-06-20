use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};

use super::AppServerTransport;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub struct WsTransport {
    stream: WsStream,
    child: Option<Child>,
}

impl WsTransport {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        ensure_loopback_ws(url)?;
        let (stream, _) = connect_async(url).await?;
        Ok(Self {
            stream,
            child: None,
        })
    }

    pub async fn start_managed() -> anyhow::Result<(String, Self)> {
        let port = pick_free_port().await?;
        let url = format!("ws://127.0.0.1:{port}");
        let child = Command::new("codex")
            .arg("app-server")
            .arg("--listen")
            .arg(&url)
            .spawn()?;
        wait_ready(port).await?;
        let (stream, _) = connect_async(&url).await?;
        Ok((
            url,
            Self {
                stream,
                child: Some(child),
            },
        ))
    }
}

#[async_trait]
impl AppServerTransport for WsTransport {
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
        if let Some(child) = &mut self.child {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

pub fn ensure_loopback_ws(url: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url)?;
    if parsed.scheme() != "ws" {
        anyhow::bail!("only ws:// endpoints are supported by WsTransport");
    }
    match parsed.host_str() {
        Some("127.0.0.1") | Some("localhost") | Some("::1") => Ok(()),
        other => anyhow::bail!("refusing non-loopback WebSocket endpoint: {:?}", other),
    }
}

async fn pick_free_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn wait_ready(port: u16) -> anyhow::Result<()> {
    let ready = format!("http://127.0.0.1:{port}/readyz");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("app-server did not become ready at {ready}");
        }
        if let Ok(Ok(_)) = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_ws() {
        assert!(ensure_loopback_ws("ws://127.0.0.1:9").is_ok());
        assert!(ensure_loopback_ws("ws://localhost:9").is_ok());
        assert!(ensure_loopback_ws("ws://192.168.1.2:9").is_err());
    }
}
