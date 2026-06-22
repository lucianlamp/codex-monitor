use async_trait::async_trait;
use serde_json::Value;

use crate::target::Endpoint;

pub mod memory;
pub mod stdio;
pub mod ws;

#[cfg(unix)]
pub mod unix;

#[async_trait]
pub trait AppServerTransport: Send {
    async fn send(&mut self, message: Value) -> anyhow::Result<()>;
    async fn recv(&mut self) -> anyhow::Result<Option<Value>>;
    async fn close(&mut self) -> anyhow::Result<()>;
}

#[async_trait]
impl<T: AppServerTransport + ?Sized> AppServerTransport for Box<T> {
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        (**self).send(message).await
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        (**self).recv().await
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        (**self).close().await
    }
}

pub async fn open_endpoint_transport(
    endpoint: crate::target::Endpoint,
) -> anyhow::Result<Box<dyn AppServerTransport>> {
    match endpoint {
        Endpoint::Auto => anyhow::bail!("auto endpoint must be resolved before opening transport"),
        Endpoint::Explicit(url) if url.starts_with("ws://") => {
            let transport = crate::transport::ws::WsTransport::connect(&url).await?;
            Ok(Box::new(transport))
        }
        Endpoint::Explicit(url) if url == "stdio://" => {
            let transport = crate::transport::stdio::StdioTransport::spawn().await?;
            Ok(Box::new(transport))
        }
        Endpoint::Explicit(url) if url.starts_with("unix://") => {
            #[cfg(unix)]
            {
                let raw_path = url
                    .strip_prefix("unix://")
                    .expect("starts_with checked above");
                if raw_path.is_empty() {
                    anyhow::bail!("unix:// endpoint requires a socket path");
                }
                let transport =
                    crate::transport::unix::UnixTransport::connect(std::path::Path::new(raw_path))
                        .await?;
                Ok(Box::new(transport))
            }
            #[cfg(not(unix))]
            {
                anyhow::bail!("unix:// endpoints require Unix socket support on this platform")
            }
        }
        Endpoint::Managed => {
            let (_url, transport) = crate::transport::ws::WsTransport::start_managed().await?;
            Ok(Box::new(transport))
        }
        Endpoint::App => {
            #[cfg(unix)]
            {
                let transport = crate::transport::unix::UnixTransport::connect(
                    &crate::target::default_app_socket_path(),
                )
                .await?;
                Ok(Box::new(transport))
            }
            #[cfg(not(unix))]
            {
                anyhow::bail!("--target app requires Unix socket support on this platform")
            }
        }
        Endpoint::Explicit(url) => anyhow::bail!("unsupported endpoint: {url}"),
    }
}
