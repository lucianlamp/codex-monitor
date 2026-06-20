use std::collections::HashMap;

use anyhow::{anyhow, bail};
use serde_json::Value;

use crate::protocol::{self, Incoming};
use crate::transport::AppServerTransport;

pub struct AppServerClient<T> {
    transport: T,
    next_id: u64,
    pending: HashMap<u64, String>,
}

impl<T: AppServerTransport> AppServerClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: 1,
            pending: HashMap::new(),
        }
    }

    pub fn into_inner(self) -> T {
        self.transport
    }

    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        let id = self.alloc_id("initialize");
        self.transport.send(protocol::initialize(id)).await?;
        self.wait_response(id, "initialize").await?;
        self.transport.send(protocol::initialized()).await?;
        Ok(())
    }

    pub async fn thread_list_by_cwd(&mut self, cwd: &str) -> anyhow::Result<Value> {
        let id = self.alloc_id("thread/list");
        self.transport
            .send(protocol::thread_list_by_cwd(id, cwd, 20))
            .await?;
        self.wait_response(id, "thread/list").await
    }

    pub async fn thread_read(
        &mut self,
        thread_id: &str,
        include_turns: bool,
    ) -> anyhow::Result<Value> {
        let id = self.alloc_id("thread/read");
        self.transport
            .send(protocol::thread_read(id, thread_id, include_turns))
            .await?;
        self.wait_response(id, "thread/read").await
    }

    pub async fn turn_start_and_wait(&mut self, thread_id: &str, text: &str) -> anyhow::Result<()> {
        let id = self.alloc_id("turn/start");
        self.transport
            .send(protocol::turn_start(id, thread_id, text))
            .await?;
        self.wait_response(id, "turn/start").await?;
        loop {
            let Some(value) = self.transport.recv().await? else {
                bail!("transport closed before turn completed");
            };
            match protocol::classify(&value) {
                Incoming::Notification { method } if method == "turn/completed" => {
                    let status = value
                        .get("params")
                        .and_then(|params| params.get("turn"))
                        .and_then(|turn| turn.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or("completed");
                    if status == "completed" {
                        return Ok(());
                    }
                    bail!("turn completed with failure status: {status}");
                }
                Incoming::Notification { .. } | Incoming::Unknown => {}
                Incoming::ServerRequest { method, .. } => {
                    bail!("server request requires human action: {method}");
                }
                Incoming::Response { id, result, error } => {
                    self.finish_response(id, result, error)?;
                }
            }
        }
    }

    fn alloc_id(&mut self, method: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, method.to_string());
        id
    }

    async fn wait_response(
        &mut self,
        expected_id: u64,
        expected_method: &str,
    ) -> anyhow::Result<Value> {
        loop {
            let Some(value) = self.transport.recv().await? else {
                bail!("transport closed while waiting for {expected_method}");
            };
            match protocol::classify(&value) {
                Incoming::Response { id, result, error } if id == expected_id => {
                    return self.finish_response(id, result, error);
                }
                Incoming::Response { id, result, error } => {
                    self.finish_response(id, result, error)?;
                }
                Incoming::ServerRequest { method, .. } => {
                    bail!("server request requires human action: {method}");
                }
                Incoming::Notification { .. } | Incoming::Unknown => {}
            }
        }
    }

    fn finish_response(
        &mut self,
        id: u64,
        result: Option<Value>,
        error: Option<Value>,
    ) -> anyhow::Result<Value> {
        let method = self
            .pending
            .remove(&id)
            .unwrap_or_else(|| format!("request {id}"));
        if let Some(error) = error {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("JSON-RPC error");
            bail!("{method}: {message}");
        }
        result.ok_or_else(|| anyhow!("{method}: response missing result"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::memory::MemoryTransport;
    use serde_json::json;

    #[tokio::test]
    async fn initialize_sends_initialize_and_initialized() {
        let inbound = vec![json!({ "id": 1, "result": { "serverInfo": { "name": "fake" } } })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.initialize().await.unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "initialize");
        assert_eq!(
            transport.sent[0]["params"]["clientInfo"]["name"],
            "codex-control-bridge"
        );
        assert_eq!(
            transport.sent[1],
            json!({ "method": "initialized", "params": {} })
        );
    }

    #[tokio::test]
    async fn thread_list_sends_cwd_filter() {
        let inbound = vec![json!({ "id": 1, "result": {} })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.thread_list_by_cwd("/tmp/project").await.unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "thread/list");
        assert_eq!(transport.sent[0]["params"]["cwd"], "/tmp/project");
    }

    #[tokio::test]
    async fn thread_read_sends_include_turns() {
        let inbound = vec![json!({ "id": 1, "result": { "thread": { "id": "thread-1" } } })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.thread_read("thread-1", true).await.unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "thread/read");
        assert_eq!(transport.sent[0]["params"]["threadId"], "thread-1");
        assert_eq!(transport.sent[0]["params"]["includeTurns"], true);
    }

    #[tokio::test]
    async fn turn_start_waits_for_terminal_completion() {
        let inbound = vec![
            json!({ "id": 1, "result": { "turn": { "id": "turn-1" } } }),
            json!({ "method": "turn/started", "params": { "turn": { "id": "turn-1" } } }),
            json!({ "method": "turn/completed", "params": { "turn": { "id": "turn-1", "status": "completed" } } }),
        ];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client
            .turn_start_and_wait("thread-1", "hello")
            .await
            .unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "turn/start");
    }

    #[tokio::test]
    async fn server_request_is_refused() {
        let inbound = vec![
            json!({ "id": 1, "result": { "turn": { "id": "turn-1" } } }),
            json!({ "id": 9, "method": "approval/request", "params": {} }),
        ];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        let error = client
            .turn_start_and_wait("thread-1", "hello")
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("server request requires human action: approval/request"));
    }
}
