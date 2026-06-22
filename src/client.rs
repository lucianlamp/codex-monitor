use std::collections::HashMap;

use anyhow::{anyhow, bail};
use serde_json::Value;

use crate::protocol::{self, Incoming};
use crate::remote_control::{
    RemoteControlClient, RemoteControlPairingStart, RemoteControlPairingStatus, RemoteControlStatus,
};
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

    pub async fn close(&mut self) -> anyhow::Result<()> {
        self.transport.close().await
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

    pub async fn thread_loaded_list(&mut self) -> anyhow::Result<Vec<String>> {
        let id = self.alloc_id("thread/loaded/list");
        self.transport
            .send(protocol::thread_loaded_list(id, 100))
            .await?;
        let result = self.wait_response(id, "thread/loaded/list").await?;
        crate::target::parse_loaded_thread_list(&result)
    }

    pub async fn ensure_thread_loaded(&mut self, thread_id: &str) -> anyhow::Result<()> {
        let loaded_threads = self.thread_loaded_list().await?;
        if loaded_threads
            .iter()
            .any(|loaded_thread| loaded_thread == thread_id)
        {
            return Ok(());
        }

        let loaded = if loaded_threads.is_empty() {
            "none".to_string()
        } else {
            loaded_threads.join(", ")
        };
        bail!(
            "thread {thread_id} is not loaded in the target app-server; refusing to call thread/resume because it can fork. Loaded threads: {loaded}"
        );
    }

    pub async fn remote_control_status_read(&mut self) -> anyhow::Result<RemoteControlStatus> {
        let id = self.alloc_id("remoteControl/status/read");
        self.transport
            .send(protocol::remote_control_status_read(id))
            .await?;
        let result = self.wait_response(id, "remoteControl/status/read").await?;
        crate::remote_control::parse_status(&result)
    }

    pub async fn remote_control_enable(&mut self) -> anyhow::Result<RemoteControlStatus> {
        let id = self.alloc_id("remoteControl/enable");
        self.transport
            .send(protocol::remote_control_enable(id))
            .await?;
        let result = self.wait_response(id, "remoteControl/enable").await?;
        crate::remote_control::parse_status(&result)
    }

    pub async fn remote_control_disable(&mut self) -> anyhow::Result<RemoteControlStatus> {
        let id = self.alloc_id("remoteControl/disable");
        self.transport
            .send(protocol::remote_control_disable(id))
            .await?;
        let result = self.wait_response(id, "remoteControl/disable").await?;
        crate::remote_control::parse_status(&result)
    }

    pub async fn remote_control_pairing_start(
        &mut self,
        manual_code: bool,
    ) -> anyhow::Result<RemoteControlPairingStart> {
        let id = self.alloc_id("remoteControl/pairing/start");
        self.transport
            .send(protocol::remote_control_pairing_start(id, manual_code))
            .await?;
        let result = self
            .wait_response(id, "remoteControl/pairing/start")
            .await?;
        crate::remote_control::parse_pairing_start(&result)
    }

    pub async fn remote_control_pairing_status(
        &mut self,
        pairing_code: Option<&str>,
        manual_pairing_code: Option<&str>,
    ) -> anyhow::Result<RemoteControlPairingStatus> {
        let id = self.alloc_id("remoteControl/pairing/status");
        self.transport
            .send(protocol::remote_control_pairing_status(
                id,
                pairing_code,
                manual_pairing_code,
            ))
            .await?;
        let result = self
            .wait_response(id, "remoteControl/pairing/status")
            .await?;
        crate::remote_control::parse_pairing_status(&result)
    }

    pub async fn remote_control_clients_list(
        &mut self,
        environment_id: &str,
    ) -> anyhow::Result<Vec<RemoteControlClient>> {
        let id = self.alloc_id("remoteControl/client/list");
        self.transport
            .send(protocol::remote_control_clients_list(
                id,
                environment_id,
                100,
            ))
            .await?;
        let result = self.wait_response(id, "remoteControl/client/list").await?;
        crate::remote_control::parse_clients(&result)
    }

    pub async fn account_read(&mut self, refresh_token: bool) -> anyhow::Result<Value> {
        let id = self.alloc_id("account/read");
        self.transport
            .send(protocol::account_read(id, refresh_token))
            .await?;
        self.wait_response(id, "account/read").await
    }

    pub async fn active_turn_id(&mut self, thread_id: &str) -> anyhow::Result<Option<String>> {
        let result = self.thread_read(thread_id, true).await?;
        Ok(active_turn_id_from_thread_read(&result))
    }

    pub async fn turn_start(&mut self, thread_id: &str, text: &str) -> anyhow::Result<String> {
        let id = self.alloc_id("turn/start");
        self.transport
            .send(protocol::turn_start(id, thread_id, text))
            .await?;
        let result = self.wait_response(id, "turn/start").await?;
        Ok(result
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub async fn turn_steer(
        &mut self,
        thread_id: &str,
        expected_turn_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let id = self.alloc_id("turn/steer");
        self.transport
            .send(protocol::turn_steer(id, thread_id, expected_turn_id, text))
            .await?;
        self.wait_response(id, "turn/steer").await?;
        Ok(())
    }

    pub async fn turn_start_or_steer(
        &mut self,
        thread_id: &str,
        text: &str,
        expected_turn_id: Option<String>,
    ) -> anyhow::Result<()> {
        let active_turn = match expected_turn_id {
            Some(turn_id) => Some(turn_id),
            None => self.active_turn_id(thread_id).await?,
        };
        if let Some(active_turn) = active_turn {
            if self.turn_steer(thread_id, &active_turn, text).await.is_ok() {
                return Ok(());
            }
        }
        self.turn_start(thread_id, text).await?;
        Ok(())
    }

    pub async fn turn_start_and_wait(&mut self, thread_id: &str, text: &str) -> anyhow::Result<()> {
        let turn_id = self.turn_start(thread_id, text).await?;
        let expected_turn_id = if turn_id.is_empty() {
            None
        } else {
            Some(turn_id.as_str())
        };
        self.wait_turn_completed(expected_turn_id).await
    }

    pub async fn turn_steer_and_wait(
        &mut self,
        thread_id: &str,
        expected_turn_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        self.turn_steer(thread_id, expected_turn_id, text).await?;
        self.wait_turn_completed(Some(expected_turn_id)).await
    }

    async fn wait_turn_completed(&mut self, expected_turn_id: Option<&str>) -> anyhow::Result<()> {
        loop {
            let Some(value) = self.transport.recv().await? else {
                bail!("transport closed before turn completed");
            };
            match protocol::classify(&value) {
                Incoming::Notification { method } if method == "turn/completed" => {
                    let turn_id = value
                        .get("params")
                        .and_then(|params| params.get("turn"))
                        .and_then(|turn| turn.get("id"))
                        .and_then(Value::as_str);
                    if expected_turn_id.is_some()
                        && turn_id.is_some()
                        && turn_id != expected_turn_id
                    {
                        continue;
                    }
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

fn active_turn_id_from_thread_read(value: &Value) -> Option<String> {
    let thread = value.get("thread")?;
    let is_active = thread
        .get("status")
        .and_then(|status| status.get("type"))
        .and_then(Value::as_str)
        == Some("active");
    if !is_active {
        return None;
    }
    thread
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
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
            "codex-monitor"
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
    async fn ensure_thread_loaded_accepts_loaded_thread() {
        let inbound = vec![json!({ "id": 1, "result": { "data": ["thread-1"] } })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.ensure_thread_loaded("thread-1").await.unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "thread/loaded/list");
    }

    #[tokio::test]
    async fn ensure_thread_loaded_refuses_to_resume_missing_thread() {
        let inbound = vec![json!({ "id": 1, "result": { "data": [] } })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        let error = client.ensure_thread_loaded("thread-1").await.unwrap_err();
        assert!(error.to_string().contains("refusing to call thread/resume"));
    }

    #[tokio::test]
    async fn remote_control_status_read_sends_expected_method() {
        let inbound = vec![json!({
            "id": 1,
            "result": {
                "status": "connected",
                "serverName": "mac.local",
                "installationId": "install-1",
                "environmentId": "env-1"
            }
        })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        let status = client.remote_control_status_read().await.unwrap();
        assert_eq!(status.status, "connected");
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "remoteControl/status/read");
    }

    #[tokio::test]
    async fn remote_control_enable_sends_unit_params() {
        let inbound = vec![json!({
            "id": 1,
            "result": {
                "status": "connected",
                "serverName": "mac.local",
                "installationId": "install-1",
                "environmentId": "env-1"
            }
        })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.remote_control_enable().await.unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "remoteControl/enable");
        assert!(transport.sent[0]["params"].is_null());
    }

    #[tokio::test]
    async fn remote_control_clients_list_sends_environment_id() {
        let inbound = vec![json!({
            "id": 1,
            "result": {
                "data": [{ "clientId": "client-1", "displayName": "Phone" }],
                "nextCursor": null
            }
        })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        let clients = client.remote_control_clients_list("env-1").await.unwrap();
        assert_eq!(clients[0].client_id, "client-1");
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "remoteControl/client/list");
        assert_eq!(transport.sent[0]["params"]["environmentId"], "env-1");
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
    async fn turn_start_returns_after_ack_without_waiting_for_completion() {
        let inbound = vec![json!({ "id": 1, "result": { "turn": { "id": "turn-1" } } })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        let turn_id = client.turn_start("thread-1", "hello").await.unwrap();
        assert_eq!(turn_id, "turn-1");
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "turn/start");
    }

    #[tokio::test]
    async fn active_turn_id_reads_latest_in_progress_turn() {
        let inbound = vec![json!({
            "id": 1,
            "result": {
                "thread": {
                    "id": "thread-1",
                    "status": { "type": "active", "activeFlags": [] },
                    "turns": [
                        { "id": "turn-old", "status": "completed", "items": [] },
                        { "id": "turn-active", "status": "inProgress", "items": [] }
                    ]
                }
            }
        })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        let turn_id = client.active_turn_id("thread-1").await.unwrap();
        assert_eq!(turn_id.as_deref(), Some("turn-active"));
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "thread/read");
        assert_eq!(transport.sent[0]["params"]["includeTurns"], true);
    }

    #[tokio::test]
    async fn turn_steer_sends_expected_turn_id() {
        let inbound = vec![json!({ "id": 1, "result": {} })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client
            .turn_steer("thread-1", "turn-active", "hello")
            .await
            .unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "turn/steer");
        assert_eq!(transport.sent[0]["params"]["expectedTurnId"], "turn-active");
    }

    #[tokio::test]
    async fn turn_start_or_steer_uses_active_turn_without_waiting_for_completion() {
        let inbound = vec![
            json!({
                "id": 1,
                "result": {
                    "thread": {
                        "id": "thread-1",
                        "status": { "type": "active", "activeFlags": [] },
                        "turns": [
                            { "id": "turn-active", "status": "inProgress", "items": [] }
                        ]
                    }
                }
            }),
            json!({ "id": 2, "result": {} }),
        ];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client
            .turn_start_or_steer("thread-1", "hello", None)
            .await
            .unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "thread/read");
        assert_eq!(transport.sent[1]["method"], "turn/steer");
        assert_eq!(transport.sent[1]["params"]["expectedTurnId"], "turn-active");
    }

    #[tokio::test]
    async fn turn_start_or_steer_falls_back_to_start_when_idle() {
        let inbound = vec![
            json!({
                "id": 1,
                "result": {
                    "thread": {
                        "id": "thread-1",
                        "status": { "type": "idle" },
                        "turns": []
                    }
                }
            }),
            json!({ "id": 2, "result": { "turn": { "id": "turn-started" } } }),
        ];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client
            .turn_start_or_steer("thread-1", "hello", None)
            .await
            .unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "thread/read");
        assert_eq!(transport.sent[1]["method"], "turn/start");
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
