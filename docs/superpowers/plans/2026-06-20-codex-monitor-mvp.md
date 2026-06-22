# Codex Monitor MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first working `codex-monitor` / `cdxm` CLI that can list Codex app-server threads, send turns, and watch agmsg events through a source-adapter boundary.

**Architecture:** The Rust library owns protocol builders, request/response handling, target resolution, transport abstraction, local state, delivery policy, and source adapters. The primary binary and alias binary both call the same library entrypoint. The MVP defaults to a cdxm-managed loopback WebSocket target, supports Unix App attach behind `cfg(unix)`, keeps stdio available for isolated use, and keeps agmsg code under `sources::agmsg`.

**Tech Stack:** Rust 2021, tokio, clap derive, serde/serde_json, tokio-tungstenite, futures-util, rusqlite, directories, tempfile. Context7 checked `/snapview/tokio-tungstenite` for `connect_async`, `client_async`, `WebSocketStream::split`, `SinkExt`, and `StreamExt`; Context7 checked `/clap-rs/clap` for derive `Parser`, `Subcommand`, command naming, and enum dispatch. Crate registry versions checked on 2026-06-20: `tokio-tungstenite 0.29.0`, `tokio 1.52.3`, `clap 4.6.1`, `rusqlite 0.40.1`, `directories 6.0.0`, `futures-util 0.3.32`, `serde 1.0.228`, `serde_json 1.0.150`, `thiserror 2.0.18`, `anyhow 1.0.102`, `url 2.5.8`, `http 1.4.2`, `async-trait 0.1.89`, `tempfile 3.27.0`.

---

## File Structure

- Create `.gitignore`: Rust build output and local run artifacts.
- Create `Cargo.toml`: package name, binaries, dependencies, dev-dependencies.
- Create `README.md`: command summary, target realm defaults, safety notes.
- Create `src/lib.rs`: public module tree and `VERSION`.
- Create `src/main.rs`: primary binary entrypoint.
- Create `src/bin/cdxm.rs`: alias binary entrypoint.
- Create `src/cli.rs`: clap parser and command dispatch shell.
- Create `src/protocol.rs`: app-server JSON-RPC message builders and classifiers.
- Create `src/client.rs`: app-server client using a transport object.
- Create `src/transport/mod.rs`: transport trait and shared message helpers.
- Create `src/transport/memory.rs`: test transport.
- Create `src/transport/ws.rs`: loopback WebSocket transport and managed child startup.
- Create `src/transport/stdio.rs`: stdio transport.
- Create `src/transport/unix.rs`: Unix WebSocket-over-Unix transport behind `cfg(unix)`.
- Create `src/target.rs`: endpoint selection and thread resolver.
- Create `src/delivery.rs`: source event formatting, per-thread sequential delivery.
- Create `src/state.rs`: JSON state file and lock file.
- Create `src/sources/mod.rs`: source adapter trait and `BridgeEvent`.
- Create `src/sources/agmsg.rs`: agmsg SQLite polling adapter.
- Create `tests/cli_contract.rs`: binary naming and CLI behavior tests.
- Create `tests/fake_app_server.rs`: loopback WebSocket fake app-server integration tests.
- Create `tests/agmsg_adapter.rs`: fixture SQLite tests.

---

### Task 1: Scaffold Cargo Project and Naming Contract

**Files:**
- Create: `.gitignore`
- Create: `Cargo.toml`
- Create: `README.md`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/bin/cdxm.rs`
- Create: `tests/cli_contract.rs`

- [ ] **Step 1: Create the Cargo scaffold and naming test**

Create `Cargo.toml`:

```toml
[package]
name = "codex-monitor"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "Local-first bridge for delivering external events into Codex app-server."

[lib]
name = "codex_monitor"
path = "src/lib.rs"

[[bin]]
name = "codex-monitor"
path = "src/main.rs"

[[bin]]
name = "cdxm"
path = "src/bin/cdxm.rs"

[dependencies]
anyhow = "1.0.102"
async-trait = "0.1.89"
clap = { version = "4.6.1", features = ["derive", "env"] }
directories = "6.0.0"
futures-util = "0.3.32"
http = "1.4.2"
rusqlite = { version = "0.40.1", features = ["bundled"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
thiserror = "2.0.18"
tokio = { version = "1.52.3", features = ["fs", "io-util", "macros", "net", "process", "rt-multi-thread", "signal", "sync", "time"] }
tokio-tungstenite = "0.29.0"
url = "2.5.8"

[dev-dependencies]
tempfile = "3.27.0"
```

Create `.gitignore`:

```gitignore
/target/
/.cdxm-state/
*.log
```

Create `src/lib.rs`:

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CLIENT_INFO_NAME: &str = "codex-monitor";
pub const CLIENT_INFO_TITLE: &str = "Codex Monitor";

pub mod cli;

pub async fn run_cli() -> anyhow::Result<i32> {
    cli::run_from_env().await
}
```

Create `src/main.rs`:

```rust
#[tokio::main]
async fn main() {
    let code = match codex_monitor::run_cli().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}
```

Create `src/bin/cdxm.rs`:

```rust
#[tokio::main]
async fn main() {
    let code = match codex_monitor::run_cli().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}
```

Create `src/cli.rs`:

```rust
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(name = "codex-monitor")]
#[command(about = "Local-first bridge for Codex app-server control plane events.")]
pub struct Cli {
    #[arg(long, global = true)]
    pub endpoint: Option<String>,

    #[arg(long, global = true, value_enum, default_value_t = TargetKind::Managed)]
    pub target: TargetKind,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum TargetKind {
    Managed,
    App,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    Threads {
        #[arg(long)]
        cwd: String,
    },
    Send {
        #[arg(long)]
        thread: String,
        #[arg(long)]
        text: String,
    },
    Agmsg {
        #[command(subcommand)]
        command: AgmsgCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgmsgCommand {
    Watch {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        thread: String,
        #[arg(long)]
        agmsg_db: Option<String>,
    },
}

pub async fn run_from_env() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    run(cli).await
}

pub async fn run(_cli: Cli) -> anyhow::Result<i32> {
    Ok(0)
}
```

Create `tests/cli_contract.rs`:

```rust
use std::process::Command;

#[test]
fn package_exposes_primary_and_alias_binaries() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");
    let alias = env!("CARGO_BIN_EXE_cdxm");

    let primary_output = Command::new(primary).arg("--help").output().unwrap();
    let alias_output = Command::new(alias).arg("--help").output().unwrap();

    assert!(primary_output.status.success());
    assert!(alias_output.status.success());

    let primary_help = String::from_utf8(primary_output.stdout).unwrap();
    let alias_help = String::from_utf8(alias_output.stdout).unwrap();

    assert!(primary_help.contains("codex-monitor"));
    assert!(primary_help.contains("threads"));
    assert!(primary_help.contains("send"));
    assert!(primary_help.contains("agmsg"));
    assert!(alias_help.contains("codex-monitor"));
}

#[test]
fn client_info_name_contract_is_fixed() {
    assert_eq!(codex_monitor::CLIENT_INFO_NAME, "codex-monitor");
    assert_eq!(codex_monitor::CLIENT_INFO_TITLE, "Codex Monitor");
}
```

Create `README.md`:

```markdown
# codex-monitor

`codex-monitor` is a local-first bridge for delivering external events
into the Codex App / Codex app-server control plane.

The short alias binary is `cdxm`.

MVP commands:

```bash
cdxm threads --cwd <path>
cdxm send --thread <id> --text <msg>
cdxm agmsg watch --team <team> --name <agent> --thread <id>
```

Default target is a bridge-managed loopback app-server. Existing Codex App UI
attach is explicit with `--target app` or an explicit `unix://` endpoint.

The core bridge is source-agnostic. agmsg is the first source adapter.
```

- [ ] **Step 2: Run tests to verify scaffold**

Run:

```bash
cargo test --test cli_contract
```

Expected: PASS. This is a scaffold task, so the test should already pass after the files above exist.

- [ ] **Step 3: Format and commit**

Run:

```bash
cargo fmt --check
git add .gitignore Cargo.toml README.md src tests
git commit -m "chore: scaffold codex-monitor crate"
```

Expected: commit succeeds.

---

### Task 2: Protocol Builders and Message Classification

**Files:**
- Modify: `src/lib.rs`
- Create: `src/protocol.rs`
- Test: `src/protocol.rs`

- [ ] **Step 1: Expose the protocol module**

Modify `src/lib.rs`:

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CLIENT_INFO_NAME: &str = "codex-monitor";
pub const CLIENT_INFO_TITLE: &str = "Codex Monitor";

pub mod cli;
pub mod protocol;

pub async fn run_cli() -> anyhow::Result<i32> {
    cli::run_from_env().await
}
```

- [ ] **Step 2: Write protocol tests first**

Create `src/protocol.rs` with tests and stubs:

```rust
use serde_json::{json, Value};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Incoming {
    Response { id: u64, result: Option<Value>, error: Option<Value> },
    Notification { method: String },
    ServerRequest { id: u64, method: String },
    Unknown,
}

pub fn initialize(_id: u64) -> Value {
    json!({})
}

pub fn initialized() -> Value {
    json!({})
}

pub fn thread_list_by_cwd(_id: u64, _cwd: &str, _limit: u32) -> Value {
    json!({})
}

pub fn thread_read(_id: u64, _thread_id: &str, _include_turns: bool) -> Value {
    json!({})
}

pub fn turn_start(_id: u64, _thread_id: &str, _text: &str) -> Value {
    json!({})
}

pub fn thread_inject_items(_id: u64, _thread_id: &str, _items: Vec<Value>) -> Value {
    json!({})
}

pub fn classify(_value: &Value) -> Incoming {
    Incoming::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_has_fixed_client_info() {
        let v = initialize(1);
        assert_eq!(v["method"], "initialize");
        assert_eq!(v["id"], 1);
        assert_eq!(v["params"]["clientInfo"]["name"], "codex-monitor");
        assert_eq!(v["params"]["clientInfo"]["title"], "Codex Monitor");
        assert_eq!(v["params"]["clientInfo"]["version"], "0.1.0");
        assert_eq!(v["params"]["capabilities"]["experimentalApi"], true);
    }

    #[test]
    fn initialized_is_notification() {
        let v = initialized();
        assert_eq!(v, json!({ "method": "initialized", "params": {} }));
    }

    #[test]
    fn thread_list_uses_cwd_filter_and_limit() {
        let v = thread_list_by_cwd(2, "/tmp/project", 20);
        assert_eq!(v["method"], "thread/list");
        assert_eq!(v["params"]["cwd"], "/tmp/project");
        assert_eq!(v["params"]["limit"], 20);
        assert_eq!(v["params"]["sortDirection"], "desc");
    }

    #[test]
    fn thread_read_can_include_turns() {
        let v = thread_read(3, "thread-1", true);
        assert_eq!(v["method"], "thread/read");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["includeTurns"], true);
    }

    #[test]
    fn turn_start_wraps_text_input() {
        let v = turn_start(4, "thread-1", "hello");
        assert_eq!(v["method"], "turn/start");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["input"][0]["type"], "text");
        assert_eq!(v["params"]["input"][0]["text"], "hello");
    }

    #[test]
    fn inject_items_is_explicit_raw_append() {
        let item = json!({ "type": "message", "role": "user", "content": "x" });
        let v = thread_inject_items(5, "thread-1", vec![item.clone()]);
        assert_eq!(v["method"], "thread/inject_items");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["items"], json!([item]));
    }

    #[test]
    fn classify_response_notification_and_server_request() {
        assert_eq!(
            classify(&json!({ "id": 1, "result": { "ok": true } })),
            Incoming::Response { id: 1, result: Some(json!({ "ok": true })), error: None }
        );
        assert_eq!(
            classify(&json!({ "method": "turn/completed", "params": {} })),
            Incoming::Notification { method: "turn/completed".to_string() }
        );
        assert_eq!(
            classify(&json!({ "id": 7, "method": "approval/request", "params": {} })),
            Incoming::ServerRequest { id: 7, method: "approval/request".to_string() }
        );
    }
}
```

- [ ] **Step 3: Run test to verify RED**

Run:

```bash
cargo test --lib protocol::tests
```

Expected: FAIL on the protocol assertions because builders return empty JSON.

- [ ] **Step 4: Implement the builders and classifier**

Replace the stub function bodies in `src/protocol.rs` with:

```rust
pub fn initialize(id: u64) -> Value {
    json!({
        "method": "initialize",
        "id": id,
        "params": {
            "clientInfo": {
                "name": crate::CLIENT_INFO_NAME,
                "title": crate::CLIENT_INFO_TITLE,
                "version": crate::VERSION
            },
            "capabilities": { "experimentalApi": true }
        }
    })
}

pub fn initialized() -> Value {
    json!({ "method": "initialized", "params": {} })
}

pub fn thread_list_by_cwd(id: u64, cwd: &str, limit: u32) -> Value {
    json!({
        "method": "thread/list",
        "id": id,
        "params": {
            "cwd": cwd,
            "limit": limit,
            "sortDirection": "desc"
        }
    })
}

pub fn thread_read(id: u64, thread_id: &str, include_turns: bool) -> Value {
    json!({
        "method": "thread/read",
        "id": id,
        "params": {
            "threadId": thread_id,
            "includeTurns": include_turns
        }
    })
}

pub fn turn_start(id: u64, thread_id: &str, text: &str) -> Value {
    json!({
        "method": "turn/start",
        "id": id,
        "params": {
            "threadId": thread_id,
            "input": [{ "type": "text", "text": text }]
        }
    })
}

pub fn thread_inject_items(id: u64, thread_id: &str, items: Vec<Value>) -> Value {
    json!({
        "method": "thread/inject_items",
        "id": id,
        "params": {
            "threadId": thread_id,
            "items": items
        }
    })
}

pub fn classify(value: &Value) -> Incoming {
    let id = value.get("id").and_then(Value::as_u64);
    let method = value.get("method").and_then(Value::as_str).map(str::to_owned);
    match (id, method) {
        (Some(id), Some(method)) => Incoming::ServerRequest { id, method },
        (Some(id), None) => Incoming::Response {
            id,
            result: value.get("result").cloned(),
            error: value.get("error").cloned(),
        },
        (None, Some(method)) => Incoming::Notification { method },
        (None, None) => Incoming::Unknown,
    }
}
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test --lib protocol::tests
cargo fmt --check
git add src/lib.rs src/protocol.rs
git commit -m "feat: add app-server protocol builders"
```

Expected: tests and format check pass.

---

### Task 3: Transport Trait, Memory Transport, and App-Server Client

**Files:**
- Modify: `src/lib.rs`
- Create: `src/transport/mod.rs`
- Create: `src/transport/memory.rs`
- Create: `src/client.rs`

- [ ] **Step 1: Expose modules and write client tests**

Modify `src/lib.rs`:

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CLIENT_INFO_NAME: &str = "codex-monitor";
pub const CLIENT_INFO_TITLE: &str = "Codex Monitor";

pub mod cli;
pub mod client;
pub mod protocol;
pub mod transport;

pub async fn run_cli() -> anyhow::Result<i32> {
    cli::run_from_env().await
}
```

Create `src/transport/mod.rs`:

```rust
use async_trait::async_trait;
use serde_json::Value;

pub mod memory;

#[async_trait]
pub trait AppServerTransport: Send {
    async fn send(&mut self, message: Value) -> anyhow::Result<()>;
    async fn recv(&mut self) -> anyhow::Result<Option<Value>>;
    async fn close(&mut self) -> anyhow::Result<()>;
}
```

Create `src/transport/memory.rs`:

```rust
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
        Self { sent: Vec::new(), inbound: inbound.into() }
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
```

Create `src/client.rs` with tests and stubs:

```rust
use std::collections::HashMap;

use anyhow::{anyhow, bail};
use serde_json::{json, Value};

use crate::protocol::{self, Incoming};
use crate::transport::AppServerTransport;

pub struct AppServerClient<T> {
    transport: T,
    next_id: u64,
    pending: HashMap<u64, String>,
}

impl<T: AppServerTransport> AppServerClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, next_id: 1, pending: HashMap::new() }
    }

    pub fn into_inner(self) -> T {
        self.transport
    }

    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    pub async fn thread_list_by_cwd(&mut self, _cwd: &str) -> anyhow::Result<Value> {
        Ok(json!({}))
    }

    pub async fn thread_read(&mut self, _thread_id: &str, _include_turns: bool) -> anyhow::Result<Value> {
        Ok(json!({}))
    }

    pub async fn turn_start_and_wait(&mut self, _thread_id: &str, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn request(&mut self, _method: &'static str, _message: Value) -> anyhow::Result<Value> {
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::memory::MemoryTransport;

    #[tokio::test]
    async fn initialize_sends_initialize_and_initialized() {
        let inbound = vec![json!({ "id": 1, "result": { "serverInfo": { "name": "fake" } } })];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.initialize().await.unwrap();
        let transport = client.into_inner();
        assert_eq!(transport.sent[0]["method"], "initialize");
        assert_eq!(transport.sent[0]["params"]["clientInfo"]["name"], "codex-monitor");
        assert_eq!(transport.sent[1], json!({ "method": "initialized", "params": {} }));
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
    async fn turn_start_waits_for_terminal_completion() {
        let inbound = vec![
            json!({ "id": 1, "result": { "turn": { "id": "turn-1" } } }),
            json!({ "method": "turn/started", "params": { "turn": { "id": "turn-1" } } }),
            json!({ "method": "turn/completed", "params": { "turn": { "id": "turn-1", "status": "completed" } } }),
        ];
        let transport = MemoryTransport::new(inbound);
        let mut client = AppServerClient::new(transport);
        client.turn_start_and_wait("thread-1", "hello").await.unwrap();
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
        let error = client.turn_start_and_wait("thread-1", "hello").await.unwrap_err();
        assert!(error.to_string().contains("server request requires human action: approval/request"));
    }
}
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test --lib client::tests
```

Expected: FAIL because client methods are stubs.

- [ ] **Step 3: Implement client request/response handling**

Replace `impl<T: AppServerTransport> AppServerClient<T>` in `src/client.rs` with:

```rust
impl<T: AppServerTransport> AppServerClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, next_id: 1, pending: HashMap::new() }
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
        self.transport.send(protocol::thread_list_by_cwd(id, cwd, 20)).await?;
        self.wait_response(id, "thread/list").await
    }

    pub async fn thread_read(&mut self, thread_id: &str, include_turns: bool) -> anyhow::Result<Value> {
        let id = self.alloc_id("thread/read");
        self.transport.send(protocol::thread_read(id, thread_id, include_turns)).await?;
        self.wait_response(id, "thread/read").await
    }

    pub async fn turn_start_and_wait(&mut self, thread_id: &str, text: &str) -> anyhow::Result<()> {
        let id = self.alloc_id("turn/start");
        self.transport.send(protocol::turn_start(id, thread_id, text)).await?;
        self.wait_response(id, "turn/start").await?;
        loop {
            let Some(value) = self.transport.recv().await? else {
                bail!("transport closed before turn completed");
            };
            match protocol::classify(&value) {
                Incoming::Notification { method } if method == "turn/completed" => {
                    let status = value
                        .get("params")
                        .and_then(|p| p.get("turn"))
                        .and_then(|t| t.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or("completed");
                    if status == "completed" {
                        return Ok(());
                    }
                    bail!("turn completed with failure status: {status}");
                }
                Incoming::Notification { .. } => {}
                Incoming::ServerRequest { method, .. } => {
                    bail!("server request requires human action: {method}");
                }
                Incoming::Response { id, result, error } => {
                    self.finish_response(id, result, error)?;
                }
                Incoming::Unknown => {}
            }
        }
    }

    fn alloc_id(&mut self, method: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, method.to_string());
        id
    }

    async fn wait_response(&mut self, expected_id: u64, expected_method: &str) -> anyhow::Result<Value> {
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

    fn finish_response(&mut self, id: u64, result: Option<Value>, error: Option<Value>) -> anyhow::Result<Value> {
        let method = self.pending.remove(&id).unwrap_or_else(|| format!("request {id}"));
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
```

Remove the unused stub `request` method.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test --lib client::tests
cargo test --lib protocol::tests
cargo fmt --check
git add src/lib.rs src/client.rs src/transport
git commit -m "feat: add app-server client over transport trait"
```

Expected: tests and format check pass.

---

### Task 4: Target Resolution and CLI Dispatch with Memory Transport

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/cli.rs`
- Create: `src/target.rs`

- [ ] **Step 1: Add target resolver tests**

Modify `src/lib.rs` to include:

```rust
pub mod target;
```

Create `src/target.rs`:

```rust
use anyhow::bail;
use serde_json::Value;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Endpoint {
    Managed,
    App,
    Explicit(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ThreadSummary {
    pub id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
}

pub fn endpoint_from_options(endpoint: Option<String>, target: crate::cli::TargetKind) -> Endpoint {
    match endpoint {
        Some(url) => Endpoint::Explicit(url),
        None if target == crate::cli::TargetKind::App => Endpoint::App,
        None => Endpoint::Managed,
    }
}

pub fn parse_thread_list(_value: &Value) -> anyhow::Result<Vec<ThreadSummary>> {
    Ok(Vec::new())
}

pub fn resolve_single_thread(threads: &[ThreadSummary]) -> anyhow::Result<String> {
    match threads {
        [thread] => Ok(thread.id.clone()),
        [] => bail!("no matching threads"),
        many => {
            let ids = many.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ");
            bail!("multiple matching threads; pass --thread explicitly: {ids}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoint_explicit_wins() {
        assert_eq!(
            endpoint_from_options(Some("ws://127.0.0.1:7777".into()), crate::cli::TargetKind::App),
            Endpoint::Explicit("ws://127.0.0.1:7777".into())
        );
        assert_eq!(endpoint_from_options(None, crate::cli::TargetKind::App), Endpoint::App);
        assert_eq!(endpoint_from_options(None, crate::cli::TargetKind::Managed), Endpoint::Managed);
    }

    #[test]
    fn parses_thread_list_shapes() {
        let value = json!({
            "threads": [
                { "id": "t1", "title": "One", "cwd": "/tmp/a" },
                { "thread": { "id": "t2", "title": "Two", "cwd": "/tmp/a" } }
            ]
        });
        let parsed = parse_thread_list(&value).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "t1");
        assert_eq!(parsed[1].id, "t2");
    }

    #[test]
    fn single_thread_resolution_rejects_ambiguous_matches() {
        let one = vec![ThreadSummary { id: "t1".into(), title: None, cwd: None }];
        assert_eq!(resolve_single_thread(&one).unwrap(), "t1");

        let two = vec![
            ThreadSummary { id: "t1".into(), title: None, cwd: None },
            ThreadSummary { id: "t2".into(), title: None, cwd: None },
        ];
        let error = resolve_single_thread(&two).unwrap_err();
        assert!(error.to_string().contains("multiple matching threads"));
    }
}
```

- [ ] **Step 2: Run target tests to verify RED**

Run:

```bash
cargo test --lib target::tests
```

Expected: FAIL because `parse_thread_list` returns an empty vector.

- [ ] **Step 3: Implement thread list parsing**

Replace `parse_thread_list` in `src/target.rs` with:

```rust
pub fn parse_thread_list(value: &Value) -> anyhow::Result<Vec<ThreadSummary>> {
    let raw_threads = value
        .get("threads")
        .or_else(|| value.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("thread/list response missing threads array"))?;

    let mut threads = Vec::new();
    for raw in raw_threads {
        let thread = raw.get("thread").unwrap_or(raw);
        let Some(id) = thread.get("id").and_then(Value::as_str) else {
            continue;
        };
        threads.push(ThreadSummary {
            id: id.to_string(),
            title: thread.get("title").and_then(Value::as_str).map(str::to_string),
            cwd: thread
                .get("cwd")
                .or_else(|| thread.get("session").and_then(|s| s.get("cwd")))
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Ok(threads)
}
```

- [ ] **Step 4: Wire CLI dispatch through temporary command summaries**

Modify `src/cli.rs`:

```rust
pub async fn run(cli: Cli) -> anyhow::Result<i32> {
    match cli.command {
        Commands::Threads { cwd } => {
            println!("threads cwd={cwd} endpoint={:?}", crate::target::endpoint_from_options(cli.endpoint, cli.target));
            Ok(0)
        }
        Commands::Send { thread, text } => {
            println!("send thread={thread} bytes={}", text.len());
            Ok(0)
        }
        Commands::Agmsg { command } => match command {
            AgmsgCommand::Watch { team, name, thread, agmsg_db } => {
                println!("agmsg watch team={team} name={name} thread={thread} agmsg_db={}", agmsg_db.unwrap_or_else(|| "-".into()));
                Ok(0)
            }
        },
    }
}
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test --lib target::tests
cargo test --test cli_contract
cargo fmt --check
git add src/lib.rs src/cli.rs src/target.rs
git commit -m "feat: add target resolution contract"
```

Expected: tests and format check pass. CLI still prints command summaries; network dispatch is Task 8.

---

### Task 5: State Store and Delivery Formatting

**Files:**
- Modify: `src/lib.rs`
- Create: `src/state.rs`
- Create: `src/sources/mod.rs`
- Create: `src/delivery.rs`

- [ ] **Step 1: Add modules and RED tests**

Modify `src/lib.rs` to include:

```rust
pub mod delivery;
pub mod sources;
pub mod state;
```

Create `src/sources/mod.rs`:

```rust
use std::collections::BTreeMap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BridgeEvent {
    pub source: String,
    pub event_id: String,
    pub observed_at: String,
    pub title: String,
    pub body: String,
    pub cwd_hint: Option<String>,
    pub reply_hint: Option<BTreeMap<String, String>>,
    pub metadata: BTreeMap<String, String>,
}

pub mod agmsg;
```

Create `src/delivery.rs` with tests and stubs:

```rust
use crate::sources::BridgeEvent;

pub fn format_event_for_turn(event: &BridgeEvent) -> String {
    event.body.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn formats_agmsg_event_with_reply_instruction() {
        let mut metadata = BTreeMap::new();
        metadata.insert("team".to_string(), "dev".to_string());
        metadata.insert("recipient".to_string(), "sally".to_string());
        metadata.insert("sender".to_string(), "kimura".to_string());
        let event = BridgeEvent {
            source: "agmsg".into(),
            event_id: "agmsg:dev:sally:1".into(),
            observed_at: "2026-06-20T00:00:00Z".into(),
            title: "agmsg from kimura".into(),
            body: "please check status".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata,
        };
        let text = format_event_for_turn(&event);
        assert!(text.contains("agmsg monitor event"));
        assert!(text.contains("Team: dev"));
        assert!(text.contains("Recipient: sally"));
        assert!(text.contains("Sender: kimura"));
        assert!(text.contains("please check status"));
        assert!(text.contains("If this requires a reply, use the agmsg scripts"));
    }
}
```

Create `src/state.rs` with tests and stubs:

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct State {
    pub delivered: BTreeMap<String, u64>,
}

pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn load(&self) -> anyhow::Result<State> {
        Ok(State::default())
    }

    pub async fn save(&self, _state: &State) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saves_and_loads_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state.json"));
        let mut state = State::default();
        state.delivered.insert("agmsg:dev:sally".into(), 42);
        store.save(&state).await.unwrap();
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded, state);
        let raw = tokio::fs::read_to_string(store.path()).await.unwrap();
        assert!(raw.contains("agmsg:dev:sally"));
    }
}
```

Create empty `src/sources/agmsg.rs`:

```rust
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test --lib delivery::tests state::tests
```

Expected: FAIL because delivery formatting and state persistence are stubs.

- [ ] **Step 3: Implement delivery formatting and state store**

Replace `format_event_for_turn` in `src/delivery.rs`:

```rust
pub fn format_event_for_turn(event: &BridgeEvent) -> String {
    if event.source == "agmsg" {
        let team = event.metadata.get("team").map(String::as_str).unwrap_or("-");
        let recipient = event.metadata.get("recipient").map(String::as_str).unwrap_or("-");
        let sender = event.metadata.get("sender").map(String::as_str).unwrap_or("-");
        return format!(
            "agmsg monitor event\n\nTeam: {team}\nRecipient: {recipient}\nSender: {sender}\n\n{}\n\nIf this requires a reply, use the agmsg scripts rather than answering only in chat.",
            event.body
        );
    }

    format!("{}\n\n{}", event.title, event.body)
}
```

Replace `load` and `save` in `src/state.rs`:

```rust
    pub async fn load(&self) -> anyhow::Result<State> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => Ok(serde_json::from_str(&raw)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn save(&self, state: &State) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(state)?;
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
```

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test --lib delivery::tests state::tests
cargo fmt --check
git add src/lib.rs src/delivery.rs src/state.rs src/sources
git commit -m "feat: add bridge event state and formatting"
```

Expected: tests and format check pass.

---

### Task 6: agmsg SQLite Source Adapter

**Files:**
- Modify: `src/sources/agmsg.rs`
- Test: `tests/agmsg_adapter.rs`

- [ ] **Step 1: Write fixture DB integration tests**

Create `tests/agmsg_adapter.rs`:

```rust
use codex_monitor::sources::agmsg::AgmsgSource;

fn create_fixture_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            team TEXT NOT NULL,
            from_agent TEXT NOT NULL,
            to_agent TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            read_at TEXT
        );
        INSERT INTO messages (team, from_agent, to_agent, body, created_at, read_at)
        VALUES
            ('dev', 'kimura', 'sally', 'first', '2026-06-20T00:00:01Z', NULL),
            ('dev', 'nakai', 'other', 'skip me', '2026-06-20T00:00:02Z', NULL),
            ('dev', 'kimura', 'sally', 'second', '2026-06-20T00:00:03Z', NULL);
        "#,
    )
    .unwrap();
}

#[test]
fn polls_matching_messages_after_last_seen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    create_fixture_db(&db_path);

    let source = AgmsgSource::new(db_path, "dev".into(), "sally".into());
    let events = source.poll_after(1).unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "agmsg:dev:sally:3");
    assert_eq!(events[0].body, "second");
    assert_eq!(events[0].metadata.get("team").unwrap(), "dev");
    assert_eq!(events[0].metadata.get("recipient").unwrap(), "sally");
    assert_eq!(events[0].metadata.get("sender").unwrap(), "kimura");
}
```

- [ ] **Step 2: Run test to verify RED**

Run:

```bash
cargo test --test agmsg_adapter
```

Expected: FAIL because `AgmsgSource` does not exist.

- [ ] **Step 3: Implement `AgmsgSource`**

Replace `src/sources/agmsg.rs` with:

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;

use rusqlite::Connection;

use crate::sources::BridgeEvent;

pub struct AgmsgSource {
    db_path: PathBuf,
    team: String,
    name: String,
}

impl AgmsgSource {
    pub fn new(db_path: PathBuf, team: String, name: String) -> Self {
        Self { db_path, team, name }
    }

    pub fn default_db_path() -> PathBuf {
        if let Ok(root) = std::env::var("AGMSG_STORAGE_PATH") {
            return PathBuf::from(root).join("messages.db");
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".agents/skills/agmsg/db/messages.db")
    }

    pub fn poll_after(&self, last_seen_id: u64) -> anyhow::Result<Vec<BridgeEvent>> {
        let conn = Connection::open(&self.db_path)?;
        let mut statement = conn.prepare(
            r#"
            SELECT id, created_at, from_agent, body
            FROM messages
            WHERE team = ?1 AND to_agent = ?2 AND id > ?3
            ORDER BY id ASC
            "#,
        )?;
        let rows = statement.query_map((&self.team, &self.name, last_seen_id), |row| {
            let id: u64 = row.get(0)?;
            let observed_at: String = row.get(1)?;
            let sender: String = row.get(2)?;
            let body: String = row.get(3)?;
            let mut metadata = BTreeMap::new();
            metadata.insert("team".to_string(), self.team.clone());
            metadata.insert("recipient".to_string(), self.name.clone());
            metadata.insert("sender".to_string(), sender.clone());
            metadata.insert("agmsg_id".to_string(), id.to_string());
            Ok(BridgeEvent {
                source: "agmsg".to_string(),
                event_id: format!("agmsg:{}:{}:{id}", self.team, self.name),
                observed_at,
                title: format!("agmsg from {sender}"),
                body,
                cwd_hint: None,
                reply_hint: None,
                metadata,
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }
}
```

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test --test agmsg_adapter
cargo test --lib delivery::tests
cargo fmt --check
git add src/sources/agmsg.rs tests/agmsg_adapter.rs
git commit -m "feat: add agmsg sqlite source adapter"
```

Expected: tests and format check pass.

---

### Task 7: WebSocket, Unix, and Stdio Transports

**Files:**
- Modify: `src/transport/mod.rs`
- Create: `src/transport/ws.rs`
- Create: `src/transport/stdio.rs`
- Create: `src/transport/unix.rs`

- [ ] **Step 1: Add transport modules**

Modify `src/transport/mod.rs`:

```rust
use async_trait::async_trait;
use serde_json::Value;

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
```

- [ ] **Step 2: Implement WebSocket transport**

Create `src/transport/ws.rs`:

```rust
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream};

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
        Ok(Self { stream, child: None })
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
        Ok((url, Self { stream, child: Some(child) }))
    }
}

#[async_trait]
impl AppServerTransport for WsTransport {
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        self.stream.send(Message::Text(message.to_string().into())).await?;
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
        ).await {
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
```

- [ ] **Step 3: Implement stdio transport**

Create `src/transport/stdio.rs`:

```rust
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::AppServerTransport;

pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
}

impl StdioTransport {
    pub async fn spawn() -> anyhow::Result<Self> {
        let mut child = Command::new("codex")
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("missing app-server stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("missing app-server stdout"))?;
        Ok(Self { child, stdin, stdout: BufReader::new(stdout).lines() })
    }
}

#[async_trait]
impl AppServerTransport for StdioTransport {
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        self.stdin.write_all(message.to_string().as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        match self.stdout.next_line().await? {
            Some(line) => Ok(Some(serde_json::from_str(&line)?)),
            None => Ok(None),
        }
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.kill().await;
        Ok(())
    }
}
```

- [ ] **Step 4: Implement Unix transport**

Create `src/transport/unix.rs`:

```rust
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
        self.stream.send(Message::Text(message.to_string().into())).await?;
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
```

- [ ] **Step 5: Verify transport compile and commit**

Run:

```bash
cargo test --lib transport::ws::tests
cargo check
cargo fmt --check
git add src/transport
git commit -m "feat: add app-server transports"
```

Expected: tests, check, and format check pass.

---

### Task 8: Real CLI Commands over Selected Transport

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/target.rs`
- Modify: `src/transport/mod.rs`
- Test: `tests/fake_app_server.rs`

- [ ] **Step 1: Write fake app-server integration tests**

Create `tests/fake_app_server.rs`:

```rust
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::process::Command;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};

async fn start_fake_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        while let Some(message) = ws.next().await {
            let Message::Text(text) = message.unwrap() else { continue };
            let request: Value = serde_json::from_str(&text).unwrap();
            match request["method"].as_str().unwrap() {
                "initialize" => {
                    ws.send(Message::Text(json!({ "id": request["id"], "result": {} }).to_string().into())).await.unwrap();
                }
                "initialized" => {}
                "thread/list" => {
                    ws.send(Message::Text(json!({
                        "id": request["id"],
                        "result": { "threads": [{ "id": "thread-1", "title": "One", "cwd": "/tmp/project" }] }
                    }).to_string().into())).await.unwrap();
                }
                "turn/start" => {
                    ws.send(Message::Text(json!({ "id": request["id"], "result": { "turn": { "id": "turn-1" } } }).to_string().into())).await.unwrap();
                    ws.send(Message::Text(json!({ "method": "turn/completed", "params": { "turn": { "id": "turn-1", "status": "completed" } } }).to_string().into())).await.unwrap();
                }
                other => panic!("unexpected method {other}"),
            }
        }
    });
    format!("ws://{}", addr)
}

#[tokio::test]
async fn threads_command_lists_fake_thread() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "threads", "--cwd", "/tmp/project"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("thread-1"));
    assert!(stdout.contains("/tmp/project"));
}

#[tokio::test]
async fn send_command_waits_for_completion() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "send", "--thread", "thread-1", "--text", "hello"])
        .output()
        .unwrap();
    assert!(output.status.success());
}
```

- [ ] **Step 2: Run integration tests to verify RED**

Run:

```bash
cargo test --test fake_app_server
```

Expected: FAIL because CLI dispatch still prints command summaries and does not connect.

- [ ] **Step 3: Implement transport selection and CLI commands**

Modify `src/target.rs` to add the App socket path:

```rust
pub fn default_app_socket_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".codex/app-server-control/app-server-control.sock")
}
```

Modify `src/transport/mod.rs` to add the boxed transport impl and endpoint opener:

```rust
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

pub async fn open_endpoint_transport(endpoint: crate::target::Endpoint) -> anyhow::Result<Box<dyn AppServerTransport>> {
    match endpoint {
        Endpoint::Explicit(url) if url.starts_with("ws://") => {
            let transport = crate::transport::ws::WsTransport::connect(&url).await?;
            Ok(Box::new(transport))
        }
        Endpoint::Explicit(url) if url == "stdio://" => {
            let transport = crate::transport::stdio::StdioTransport::spawn().await?;
            Ok(Box::new(transport))
        }
        Endpoint::Managed => {
            let (_url, transport) = crate::transport::ws::WsTransport::start_managed().await?;
            Ok(Box::new(transport))
        }
        Endpoint::App => {
            #[cfg(unix)]
            {
                let transport = crate::transport::unix::UnixTransport::connect(&default_app_socket_path()).await?;
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
```

Use these imports at the top of `src/transport/mod.rs`:

```rust
use crate::target::{default_app_socket_path, Endpoint};
```

Modify `src/cli.rs` imports:

```rust
use crate::client::AppServerClient;
use crate::target::endpoint_from_options;
use crate::transport::open_endpoint_transport;
```

Replace `run` in `src/cli.rs`:

```rust
pub async fn run(cli: Cli) -> anyhow::Result<i32> {
    let endpoint = endpoint_from_options(cli.endpoint.clone(), cli.target);
    match cli.command {
        Commands::Threads { cwd } => {
            let transport = open_endpoint_transport(endpoint).await?;
            let mut client = AppServerClient::new(transport);
            client.initialize().await?;
            let result = client.thread_list_by_cwd(&cwd).await?;
            for thread in crate::target::parse_thread_list(&result)? {
                println!(
                    "{}\t{}\t{}",
                    thread.id,
                    thread.title.unwrap_or_else(|| "-".into()),
                    thread.cwd.unwrap_or_else(|| "-".into())
                );
            }
            Ok(0)
        }
        Commands::Send { thread, text } => {
            let transport = open_endpoint_transport(endpoint).await?;
            let mut client = AppServerClient::new(transport);
            client.initialize().await?;
            client.turn_start_and_wait(&thread, &text).await?;
            Ok(0)
        }
        Commands::Agmsg { command } => match command {
            AgmsgCommand::Watch { team, name, thread, agmsg_db } => {
                crate::delivery::run_agmsg_watch(endpoint, team, name, thread, agmsg_db).await
            }
        },
    }
}
```

- [ ] **Step 4: Add delivery watch function stub for compile**

Add to `src/delivery.rs`:

```rust
pub async fn run_agmsg_watch(
    _endpoint: crate::target::Endpoint,
    _team: String,
    _name: String,
    _thread: String,
    _agmsg_db: Option<String>,
) -> anyhow::Result<i32> {
    anyhow::bail!("agmsg watch is wired in Task 9")
}
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test --test fake_app_server
cargo test --test cli_contract
cargo fmt --check
git add src/cli.rs src/target.rs src/transport/mod.rs src/delivery.rs tests/fake_app_server.rs
git commit -m "feat: wire cli to app-server transports"
```

Expected: tests and format check pass. `cdxm agmsg watch` returns the Task 9 message until implemented.

---

### Task 9: agmsg Watch Delivery Loop

**Files:**
- Modify: `src/delivery.rs`
- Modify: `src/state.rs`
- Test: `tests/agmsg_adapter.rs`

- [ ] **Step 1: Add state helper and delivery loop tests**

Extend `tests/agmsg_adapter.rs` with:

```rust
use codex_monitor::delivery::format_event_for_turn;

#[test]
fn agmsg_event_formats_for_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    create_fixture_db(&db_path);
    let source = AgmsgSource::new(db_path, "dev".into(), "sally".into());
    let events = source.poll_after(0).unwrap();
    let text = format_event_for_turn(&events[0]);
    assert!(text.contains("Team: dev"));
    assert!(text.contains("Recipient: sally"));
    assert!(text.contains("first"));
}
```

Add to `src/state.rs`:

```rust
impl State {
    pub fn last_seen(&self, key: &str) -> u64 {
        *self.delivered.get(key).unwrap_or(&0)
    }

    pub fn mark_seen(&mut self, key: String, id: u64) {
        self.delivered.insert(key, id);
    }
}
```

- [ ] **Step 2: Run test**

Run:

```bash
cargo test --test agmsg_adapter
```

Expected: PASS after the state helper compiles, because this extends already working adapter behavior.

- [ ] **Step 3: Implement `run_agmsg_watch` as a foreground polling loop**

Replace the Task 8 stub in `src/delivery.rs` with:

```rust
pub async fn run_agmsg_watch(
    endpoint: crate::target::Endpoint,
    team: String,
    name: String,
    thread: String,
    agmsg_db: Option<String>,
) -> anyhow::Result<i32> {
    let db_path = agmsg_db
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::sources::agmsg::AgmsgSource::default_db_path);
    let state_path = default_state_path()?;
    let store = crate::state::StateStore::new(state_path);
    let mut state = store.load().await?;
    let state_key = format!("agmsg:{team}:{name}");
    let source = crate::sources::agmsg::AgmsgSource::new(db_path, team, name);

    let transport = crate::transport::open_endpoint_transport(endpoint).await?;
    let mut client = crate::client::AppServerClient::new(transport);
    client.initialize().await?;

    loop {
        let last_seen = state.last_seen(&state_key);
        for event in source.poll_after(last_seen)? {
            let text = format_event_for_turn(&event);
            client.turn_start_and_wait(&thread, &text).await?;
            if let Some(raw_id) = event.metadata.get("agmsg_id").and_then(|s| s.parse::<u64>().ok()) {
                state.mark_seen(state_key.clone(), raw_id);
                store.save(&state).await?;
            }
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(0),
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }
    }
}

fn default_state_path() -> anyhow::Result<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "codex-monitor")
        .ok_or_else(|| anyhow::anyhow!("could not resolve local state directory"))?;
    Ok(dirs.state_dir().join("state.json"))
}

```

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test --test agmsg_adapter
cargo test --test fake_app_server
cargo test
cargo fmt --check
git add src/delivery.rs src/state.rs tests/agmsg_adapter.rs src/cli.rs
git commit -m "feat: deliver agmsg events to codex turns"
```

Expected: all tests and format check pass.

---

### Task 10: Cross-Platform Build Gates, README, and Final Smoke Notes

**Files:**
- Modify: `README.md`
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Update README with target modes and safety**

Replace `README.md` with:

```markdown
# codex-monitor

`codex-monitor` is a local-first bridge for delivering external events
into the Codex App / Codex app-server control plane.

The short alias binary is `cdxm`.

## Commands

```bash
cdxm threads --cwd <path>
cdxm send --thread <id> --text <msg>
cdxm agmsg watch --team <team> --name <agent> --thread <id>
```

## Targets

Default target is `managed`: cdxm starts a loopback app-server at
`ws://127.0.0.1:<port>`.

Existing Codex App UI attach is explicit:

```bash
cdxm --target app threads --cwd <path>
cdxm --target app send --thread <id> --text <msg>
```

On Unix, `--target app` attaches to:

```text
$HOME/.codex/app-server-control/app-server-control.sock
```

`--endpoint ws://127.0.0.1:<port>` connects to an explicit loopback WebSocket.
`--endpoint stdio://` starts an isolated stdio app-server.

## agmsg adapter

The agmsg adapter reads the message store directly and does not use Codex
shims, PATH replacement, SessionStart hooks, `inbox.sh`, or `watch.sh`.

Default agmsg DB:

```text
$HOME/.agents/skills/agmsg/db/messages.db
```

Override:

```bash
cdxm agmsg watch --team dev --name sally --thread <id> --agmsg-db /path/to/messages.db
```

## Safety

- cdxm never auto-approves Codex app-server requests.
- cdxm refuses non-loopback WebSocket endpoints in the MVP.
- `thread/inject_items` is not the default delivery path.
- `--target app` does not start, stop, or replace the Codex App daemon.
```

- [ ] **Step 2: Add CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
  pull_request:

jobs:
  rust:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt --check
      - run: cargo test
      - run: cargo check --target x86_64-pc-windows-msvc
        if: runner.os == 'Windows'
```

- [ ] **Step 3: Run final verification**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-pc-windows-msvc
```

Expected:

- `cargo fmt --check`: PASS
- `cargo test`: PASS
- `cargo clippy --all-targets -- -D warnings`: PASS
- `cargo check --target x86_64-pc-windows-msvc`: PASS if the target is installed; if missing, run `rustup target add x86_64-pc-windows-msvc` and retry.

- [ ] **Step 4: Manual live smoke with existing Codex App attach**

Run only if the user wants a live check against the current App daemon:

```bash
cdxm --target app threads --cwd /Users/ysk411/dev/codex-monitor
```

Expected: command initializes against the App control socket and either prints matching threads or exits 0 with no rows. Do not run `cdxm send` against a live thread without explicit user approval.

- [ ] **Step 5: Commit**

Run:

```bash
git add README.md .github/workflows/ci.yml
git commit -m "docs: add usage and ci gates"
```

Expected: commit succeeds.

---

## Plan Self-Review

Spec coverage:

- Naming contracts are covered in Task 1.
- app-server protocol builders are covered in Task 2.
- client request handling and server request refusal are covered in Task 3.
- target resolver and CLI commands are covered in Tasks 4 and 8.
- source adapter boundary and agmsg direct DB read are covered in Tasks 5, 6, and 9.
- WebSocket, Unix, and stdio transports are covered in Task 7.
- macOS App attach and Windows build constraints are covered in Tasks 7, 8, and 10.

Open-marker scan:

- No open marker entries remain in this plan.

Type consistency:

- `BridgeEvent`, `AgmsgSource`, `AppServerClient`, `AppServerTransport`, `Endpoint`, and `TargetKind` names are used consistently across tasks.
- Endpoint-to-transport mapping is defined once in `transport::open_endpoint_transport` and used by both CLI sends and agmsg watch.
