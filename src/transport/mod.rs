use async_trait::async_trait;
use serde_json::Value;

pub mod memory;

#[async_trait]
pub trait AppServerTransport: Send {
    async fn send(&mut self, message: Value) -> anyhow::Result<()>;
    async fn recv(&mut self) -> anyhow::Result<Option<Value>>;
    async fn close(&mut self) -> anyhow::Result<()>;
}
