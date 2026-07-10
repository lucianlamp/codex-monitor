# Stdio-Preserving Codex App Monitor Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the Codex App's native stdio app-server and Browser Use behavior while exposing a loopback-only minimal monitor endpoint for loaded-thread discovery and `turn/start`/`turn/steer` delivery.

**Architecture:** The bridge launches the matching real Codex app-server without a WebSocket `--listen` override and transparently proxies the App's JSONL stdio. A focused router exposes a separate loopback WebSocket listener, handles monitor initialization locally, remaps allowlisted request IDs into the App's initialized stdio session, and broadcasts only turn lifecycle notifications. The npm-backed Codex CLI path remains unchanged.

**Tech Stack:** Rust 2021, Tokio async I/O/channels/processes, tokio-tungstenite WebSockets, serde_json JSON-RPC, Windows PowerShell installer

---

## File map

- Create `src/app_bridge/monitor_router.rs`: pure JSON-RPC allowlist, readiness, ID remapping, response ownership, and notification routing.
- Create `src/app_bridge/stdio_monitor.rs`: stdio proxy, loopback WebSocket listener, bounded client queues, marker lifecycle, and owned-child shutdown.
- Modify `src/app_bridge.rs`: keep invocation/real-runtime/marker discovery, route only the Codex App signature into the new stdio monitor runtime, and remove the WebSocket child proxy.
- Modify `tests/app_bridge.rs`: process-level passthrough contract remains green; add source-level/runtime behavior where unit coverage is insufficient.
- Modify `skills/codex-monitor/SKILL.md`: document stdio preservation, minimal endpoint scope, Browser acceptance, and fail-closed Desktop CLI resolution.
- Modify `README.md`: describe the minimal App monitor endpoint and Browser-safe transport.
- Modify `docs/superpowers/specs/2026-07-10-stdio-app-monitor-bridge-design.md`: only if implementation discovery requires a clarified invariant.

### Task 1: Build the pure monitor router with TDD

**Files:**
- Create: `src/app_bridge/monitor_router.rs`
- Modify: `src/app_bridge.rs`

- [ ] **Step 1: Declare the submodule and write failing router tests**

Add `mod monitor_router;` to `src/app_bridge.rs`. Create the router file with tests that define the required public surface before implementation:

```rust
#[derive(Debug, PartialEq)]
pub(super) enum MonitorInput {
    Reply(Value),
    Forward(Value),
    Ignore,
}

#[derive(Debug, PartialEq)]
pub(super) enum ChildOutput {
    AppOnly,
    AppAndBroadcast(Value),
    Monitor { connection_id: u64, message: Value },
    Drop,
}

#[test]
fn initialize_is_local_and_allowlisted_request_waits_for_app_readiness() {
    let mut router = MonitorRouter::new("bridge-nonce");
    let init = router.handle_monitor(7, json!({"id":1,"method":"initialize","params":{}}));
    assert!(matches!(init, MonitorInput::Reply(value) if value["id"] == 1));

    let early = router.handle_monitor(7, json!({"id":2,"method":"thread/list","params":{}}));
    assert!(matches!(early, MonitorInput::Reply(value) if value["error"]["code"] == -32002));

    assert!(router.observe_app(&json!({"method":"initialized","params":{}})));
    let ready = router.handle_monitor(7, json!({"id":3,"method":"thread/list","params":{}}));
    assert!(matches!(ready, MonitorInput::Forward(value)
        if value["id"].as_str().unwrap().starts_with("cdxm:bridge-nonce:7:")));
}

#[test]
fn responses_return_only_to_the_owning_monitor_and_restore_ids() {
    let mut router = MonitorRouter::new("bridge-nonce");
    router.observe_app(&json!({"method":"initialized","params":{}}));
    let MonitorInput::Forward(forwarded) = router.handle_monitor(
        9,
        json!({"id":41,"method":"thread/read","params":{"threadId":"t"}}),
    ) else { panic!("request was not forwarded") };
    let internal = forwarded["id"].clone();
    let routed = router.route_child(&json!({"id":internal,"result":{"thread":{"id":"t"}}}));
    assert_eq!(routed, ChildOutput::Monitor {
        connection_id: 9,
        message: json!({"id":41,"result":{"thread":{"id":"t"}}}),
    });
}

#[test]
fn disallowed_methods_server_requests_and_notifications_stay_bounded() {
    let mut router = MonitorRouter::new("bridge-nonce");
    router.observe_app(&json!({"method":"initialized","params":{}}));
    let denied = router.handle_monitor(1, json!({"id":1,"method":"account/read","params":{}}));
    assert!(matches!(denied, MonitorInput::Reply(value) if value["error"]["code"] == -32601));
    assert_eq!(router.route_child(&json!({"id":8,"method":"item/tool/call","params":{}})), ChildOutput::AppOnly);
    assert!(matches!(
        router.route_child(&json!({"method":"turn/completed","params":{"turn":{"id":"x"}}})),
        ChildOutput::AppAndBroadcast(_)
    ));
    assert_eq!(
        router.route_child(&json!({"method":"thread/started","params":{}})),
        ChildOutput::AppOnly
    );
}
```

- [ ] **Step 2: Run the router tests and confirm the missing implementation fails**

Run:

```powershell
cargo test app_bridge::monitor_router::tests -- --nocapture
```

Expected: compilation fails because `MonitorRouter` and its methods do not exist.

- [ ] **Step 3: Implement the router minimally**

Implement:

```rust
pub(super) struct MonitorRouter {
    nonce: String,
    ready: bool,
    next_sequence: u64,
    pending: HashMap<String, Pending>,
}

struct Pending {
    connection_id: u64,
    original_id: Value,
}

impl MonitorRouter {
    pub(super) fn new(nonce: impl Into<String>) -> Self;
    pub(super) fn observe_app(&mut self, message: &Value) -> bool;
    pub(super) fn handle_monitor(&mut self, connection_id: u64, message: Value) -> MonitorInput;
    pub(super) fn route_child(&mut self, message: &Value) -> ChildOutput;
    pub(super) fn retire_connection(&mut self, connection_id: u64);
}
```

Use the exact request allowlist from the spec. Return JSON-RPC `-32600` for invalid shapes, `-32601` for disallowed methods, and `-32002` before App readiness. Locally answer `initialize`, ignore `initialized`, and restore the original numeric ID on monitor responses. Treat unknown internal IDs as `ChildOutput::Drop`.

- [ ] **Step 4: Run router tests and format checks**

Run:

```powershell
cargo fmt --check
cargo test app_bridge::monitor_router::tests -- --nocapture
```

Expected: all router tests pass.

- [ ] **Step 5: Commit the router**

```powershell
git add -- src/app_bridge.rs src/app_bridge/monitor_router.rs
git commit -m "feat: add minimal App monitor router"
```

### Task 2: Implement the stdio and WebSocket multiplex core

**Files:**
- Create: `src/app_bridge/stdio_monitor.rs`
- Modify: `src/app_bridge.rs`

- [ ] **Step 1: Write failing async I/O tests around duplex streams**

Expose a testable core:

```rust
pub(super) async fn proxy_stdio_monitor_io<AR, AW, CR, CW>(
    app_input: AR,
    app_output: AW,
    child_output: CR,
    child_input: CW,
    listener: TcpListener,
    nonce: String,
    ready_tx: watch::Sender<bool>,
) -> anyhow::Result<()>
where
    AR: AsyncBufRead + Unpin + Send + 'static,
    AW: AsyncWrite + Unpin + Send + 'static,
    CR: AsyncBufRead + Unpin + Send + 'static,
    CW: AsyncWrite + Unpin + Send + 'static;
```

Add `app_stdio_is_unchanged_and_monitor_requests_share_the_initialized_child`.
The test creates two `tokio::io::duplex(64 * 1024)` pairs, starts
`proxy_stdio_monitor_io`, sends an App `initialize` request and child response,
then sends App `initialized`. It connects with
`tokio_tungstenite::connect_async`, verifies monitor `initialize` is answered
without a child write, sends `thread/loaded/list`, captures its internal string
ID at the fake child, returns a response using that ID, and asserts the monitor
receives the original numeric ID while App output receives no monitor response.

Add `turn_notifications_reach_app_and_monitor_without_slow_monitor_backpressure`.
The test connects a healthy monitor and a second monitor whose output is not
read, emits `CLIENT_QUEUE_CAPACITY + 1` `turn/completed` lines from fake child,
and asserts App receives every original line, the slow client is removed, and
the healthy monitor receives the final notification.

- [ ] **Step 2: Run the focused tests and confirm failure**

Run:

```powershell
cargo test app_bridge::stdio_monitor::tests -- --nocapture
```

Expected: compilation fails because `proxy_stdio_monitor_io` is missing.

- [ ] **Step 3: Implement serialized child writes and client routing**

Implement one bounded `mpsc::channel<ChildWrite>` whose writer task alone owns child stdin:

```rust
enum ChildWrite {
    Raw(String),
    Json(Value),
}

type ClientSender = mpsc::Sender<Message>;
type Clients = Arc<Mutex<HashMap<u64, ClientSender>>>;
```

The App input task forwards the original line and calls `MonitorRouter::observe_app` only after successful JSON parsing. The child output task forwards non-monitor lines to App exactly as received, routes owned responses only to their monitor, and uses `try_send` for monitor notification broadcasts. Remove clients whose queue is full or closed.

- [ ] **Step 4: Implement the loopback WebSocket handler**

Use `tokio_tungstenite::accept_async` on accepted loopback streams. Allocate connection IDs from an `AtomicU64`, store one bounded sender per client, parse only text JSON messages, and translate `MonitorInput::{Reply,Forward,Ignore}`. On disconnect call `retire_connection` and remove the client sender. Binary messages receive `-32600` and are not forwarded.

- [ ] **Step 5: Run multiplex tests and the existing large-frame test**

Run:

```powershell
cargo test app_bridge::stdio_monitor::tests -- --nocapture
cargo test app_bridge::tests::proxy_forwards_frames_larger_than_tungstenite_default_limit -- --exact
```

Expected: new tests pass. The old large-frame test may fail because the legacy proxy is about to be removed; preserve equivalent unlimited monitor-frame coverage in the new test before deleting it.

- [ ] **Step 6: Commit the multiplex core**

```powershell
git add -- src/app_bridge.rs src/app_bridge/stdio_monitor.rs
git commit -m "feat: proxy App stdio with minimal monitor websocket"
```

### Task 3: Replace the WebSocket child bridge and enforce owned lifecycle

**Files:**
- Modify: `src/app_bridge.rs`
- Modify: `src/app_bridge/stdio_monitor.rs`
- Modify: `tests/app_bridge.rs`

- [ ] **Step 1: Replace the old rewrite test with App-signature routing tests**

Add a pure selector:

```rust
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BridgeMode { Passthrough, StdioMonitor }

fn bridge_mode(args: &[OsString]) -> BridgeMode {
    if publishes_app_target_marker(args) {
        BridgeMode::StdioMonitor
    } else {
        BridgeMode::Passthrough
    }
}
```

Test that the App signature selects `StdioMonitor`, a generic `app-server --listen` invocation selects `Passthrough`, and no argument rewrite adds `--listen`.

- [ ] **Step 2: Run focused tests and confirm old behavior fails the new contract**

Run:

```powershell
cargo test app_bridge::tests::app_signature_selects_stdio_monitor_without_rewriting_args -- --exact
```

Expected: fail until `run_bridge` uses `BridgeMode` and the legacy rewrite path is removed.

- [ ] **Step 3: Implement owned process runtime**

In `stdio_monitor::run`, bind `127.0.0.1:0`, launch the real Codex with unchanged App args, and set:

```rust
Command::new(real_codex)
    .args(args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .kill_on_drop(true)
```

Start the I/O core, wait for the App `initialized` readiness signal, then write the existing marker. On App EOF or task failure, close monitor tasks and child stdin, wait up to one second for the owned child, then call `child.kill()` only for that child if it remains alive. Remove the legacy `rewrite_app_server_args`, `wait_for_server`, client-side WebSocket proxy, and their obsolete tests.

- [ ] **Step 4: Add lifecycle tests**

Add `marker_guard_removes_only_its_owned_marker`: write two markers into a
temporary directory, drop a `MarkerGuard` for the first path, and assert only
the first file was removed. Add `app_signature_preserves_original_child_args`:
feed the App signature through `bridge_mode` and assert the exact original
argument vector is passed to the stdio runtime with no `--listen` entry.
Process ownership is covered by keeping the child handle private to
`stdio_monitor::run` and calling `kill` only through that handle; the source
audit in Task 5 rejects name-based cleanup.

Keep the existing passthrough process test in `tests/app_bridge.rs` green.

- [ ] **Step 5: Run all bridge tests**

Run:

```powershell
cargo test app_bridge -- --nocapture
cargo test --test app_bridge
```

Expected: every bridge unit/integration test passes and no test invokes a process-name kill.

- [ ] **Step 6: Commit lifecycle replacement**

```powershell
git add -- src/app_bridge.rs src/app_bridge/stdio_monitor.rs tests/app_bridge.rs
git commit -m "fix: preserve Codex App stdio transport"
```

### Task 4: Update installer contracts and operator documentation

**Files:**
- Modify: `tests/windows_installer_contract.rs`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `README.md`

- [ ] **Step 1: Add failing documentation/installer contract assertions**

Require the installed contract to mention stdio preservation, minimal methods,
Browser acceptance, and fail-closed Desktop CLI selection:

```rust
#[test]
fn docs_define_browser_safe_minimal_app_monitor_bridge() {
    let skill = fs::read_to_string(repo_root().join("skills/codex-monitor/SKILL.md")).unwrap();
    for required in [
        "native stdio",
        "thread/loaded/list",
        "turn/start",
        "turn/steer",
        "Browser",
        "refusing Windows Desktop Codex fallback",
    ] {
        assert!(skill.contains(required), "missing `{required}`");
    }
}
```

- [ ] **Step 2: Run the contract test and confirm it fails**

Run:

```powershell
cargo test --test windows_installer_contract docs_define_browser_safe_minimal_app_monitor_bridge -- --exact
```

Expected: fail because the current skill still documents Desktop as a fallback and the old shared-server bridge.

- [ ] **Step 3: Update the skill and README**

Document that the App bridge preserves native stdio and publishes a restricted loopback monitor endpoint. List the seven accepted methods, state that remote/account/tool operations are rejected, require Google Browser + DOM acceptance after install, and replace the stale Desktop fallback paragraph with the fail-closed npm behavior implemented in commit `152e93b`.

- [ ] **Step 4: Run documentation contracts**

Run:

```powershell
cargo test --test windows_installer_contract
```

Expected: all contract tests pass.

- [ ] **Step 5: Commit docs/contracts**

```powershell
git add -- tests/windows_installer_contract.rs skills/codex-monitor/SKILL.md README.md
git commit -m "docs: define Browser-safe App monitor bridge"
```

### Task 5: Complete automated verification and self-review

**Files:**
- Modify only files required by failures found in this task.

- [ ] **Step 1: Run the full verification surface**

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: all commands exit `0` with no warnings.

- [ ] **Step 2: Audit the implementation against every spec requirement**

Check each requirement in `docs/superpowers/specs/2026-07-10-stdio-app-monitor-bridge-design.md` against code or tests. Search for forbidden remnants:

```powershell
rg -n "rewrite_app_server_args|connect_async_with_config|Get-Process codex|taskkill /IM codex|pkill codex" src tests skills README.md
```

Expected: no legacy App child WebSocket proxy and no process-name cleanup. Any process-name strings may appear only in explicit prohibition documentation.

- [ ] **Step 3: Inspect the final diff for scope and ownership**

```powershell
git diff HEAD~4 --check
git diff HEAD~4 --stat
git status --short
```

Expected: only bridge, tests, docs, and planned support files changed; worktree is clean after fixes are committed.

- [ ] **Step 4: Commit verification fixes if needed**

```powershell
git add -- src/app_bridge.rs src/app_bridge/monitor_router.rs src/app_bridge/stdio_monitor.rs tests/app_bridge.rs tests/windows_installer_contract.rs skills/codex-monitor/SKILL.md README.md
git commit -m "test: harden stdio App monitor bridge"
```

### Task 6: Install and prove App, Browser, agmsg, and CLI together

**Files:**
- Installed binaries/skill under `%USERPROFILE%\.codex-monitor`, `%USERPROFILE%\.codex\skills\codex-monitor`, and `%USERPROFILE%\.agents\bin`.

- [ ] **Step 1: Build release binaries while the App remains native**

```powershell
cargo build --release --bins
```

Expected: release build succeeds without altering live processes.

- [ ] **Step 2: Install files without stopping unrelated watchers**

Copy/install only when the destination executable is not currently running. Do not stop any `cdxm` watcher. Install the skill/shim/PATH with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Yes -SkipBuild -InstallShim -Source .
```

Install the rebuilt App bridge before enabling it. Preserve `app-bridge-env.json` and `user-path-backup.json`.

- [ ] **Step 3: Enable the rebuilt bridge and restart only Codex App**

Use the owned installer path without changing the CLI shim:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Yes -SkipBuild -NoShim -NoPath -InstallAppBridge -Source .
```

Fully quit and reopen Codex App. Do not terminate any separate Codex CLI process or watcher.

- [ ] **Step 4: Prove native stdio and monitor endpoint coexist**

Inspect the current App bridge and owned child command lines. Expected:

- App parent is `cdxm-codex-app-bridge.exe`;
- child is `codex-app-real.exe ... app-server --analytics-default-enabled`;
- child command line does not contain `--listen ws://`;
- `cdxm targets` reports exactly one live `codex-app-bridge` target for the current App;
- `cdxm --target app loaded` includes the current visible thread.

- [ ] **Step 5: Prove Browser and Playwright**

Through the in-app Browser plugin, navigate once to `https://www.google.com/`, wait for `domcontentloaded`, and assert title `Google` plus a non-empty Playwright DOM snapshot. Finalize the test tab after evidence is captured.

- [ ] **Step 6: Prove agmsg delivery to the visible thread**

Use the existing `regariabrave/cdxm` identity and a unique timestamped token. Send through the agmsg scripts, verify the watcher receives it, verify `cdxm` acknowledgment targets the current App marker/thread, and confirm receipt in this visible thread. Do not use a process-name cleanup.

- [ ] **Step 7: Prove CLI isolation remains current**

```powershell
& "$HOME\.agents\bin\codex.cmd" --version
& "$env:APPDATA\npm\codex.cmd" --version
```

Expected: both report `codex-cli 0.144.1`; the Desktop `0.130.0-alpha.5` directory remains absent from the user PATH.

- [ ] **Step 8: Final completion audit**

Recheck the design's automated and live acceptance list one item at a time. Keep the bridge enabled only if Browser, loaded-thread discovery, agmsg delivery, and CLI checks all pass. Otherwise restore the native App environment and leave the failing evidence recorded without claiming completion.
