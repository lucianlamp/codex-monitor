use async_trait::async_trait;
use serde_json::Value;
use std::collections::VecDeque;

use super::AppServerTransport;

#[derive(Debug, Default)]
pub struct MemoryTransport {
    pub sent: Vec<Value>,
    inbound: VecDeque<Value>,
}

impl MemoryTransport {
    pub fn new(inbound: Vec<Value>) -> Self {
        Self {
            sent: Vec::new(),
            inbound: inbound.into(),
        }
    }
}

#[async_trait]
impl AppServerTransport for MemoryTransport {
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        self.sent.push(message);
        Ok(())
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        Ok(self.inbound.pop_front())
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
