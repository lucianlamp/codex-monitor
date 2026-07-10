# Windows Codex App Shared Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route Codex App and `cdxm` through one Windows loopback app-server so `agmsg` events reach the exact visible App thread.

**Architecture:** A new `cdxm-codex-app-bridge` binary is selected by Codex App through `CODEX_CLI_PATH`. It launches the real app-server with a loopback WebSocket, proxies App JSONL over that socket, and publishes validated per-process markers that are the only Windows `--target app` source.

**Tech Stack:** Rust, Tokio, tokio-tungstenite, serde JSON, PowerShell installer, Cargo integration tests.

---

### Task 1: Marker-backed Windows App target

**Files:**
- Create: `src/app_bridge.rs`
- Modify: `src/lib.rs`
- Modify: `src/target.rs`
- Test: `src/app_bridge.rs`
- Test: `src/target.rs`

- [ ] **Step 1: Add failing marker parsing and selection tests**

Add tests using a temporary marker directory. The test data must use the real
serialized shape and must prove a valid marker is selected while malformed,
stale, non-loopback, and multiple valid markers are rejected.

```rust
#[test]
fn app_marker_round_trips() {
    let marker = AppBridgeMarker {
        version: 1,
        endpoint: "ws://127.0.0.1:45454".into(),
        bridge_pid: 101,
        server_pid: 202,
        real_codex: PathBuf::from(r"C:\Codex\codex.exe"),
    };
    let decoded: AppBridgeMarker = serde_json::from_str(&serde_json::to_string(&marker).unwrap()).unwrap();
    assert_eq!(decoded, marker);
}

#[test]
fn windows_app_target_uses_only_bridge_markers() {
    let candidates = vec![
        EndpointCandidate { endpoint: Endpoint::Explicit("ws://127.0.0.1:1".into()), source: "codex-app-server-process".into() },
        EndpointCandidate { endpoint: Endpoint::Explicit("ws://127.0.0.1:2".into()), source: "codex-app-bridge".into() },
    ];
    assert_eq!(select_windows_app_endpoint(candidates).unwrap(), Endpoint::Explicit("ws://127.0.0.1:2".into()));
}
```

- [ ] **Step 2: Run focused tests and verify red**

Run: `cargo test app_marker`, then `cargo test windows_app_target`.

Expected: compile failure because `AppBridgeMarker` and marker discovery do not exist, followed by the existing selector choosing the process candidate.

- [ ] **Step 3: Implement marker storage and discovery**

Create `AppBridgeMarker` and these focused APIs:

```rust
pub const APP_BRIDGE_MARKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct AppBridgeMarker {
    pub version: u32,
    pub endpoint: String,
    pub bridge_pid: u32,
    pub server_pid: u32,
    pub real_codex: PathBuf,
}

pub fn marker_dir() -> PathBuf;
pub fn marker_path(dir: &Path, bridge_pid: u32) -> PathBuf;
pub fn write_marker_atomic(dir: &Path, marker: &AppBridgeMarker) -> anyhow::Result<PathBuf>;
pub fn remove_marker(path: &Path);
pub fn read_marker_candidates(dir: &Path, live_endpoints: &BTreeSet<String>) -> Vec<EndpointCandidate>;
```

`marker_dir` honors `CDXM_APP_BRIDGE_DIR` for tests and otherwise uses the
user-local codex-monitor runtime directory. `read_marker_candidates` accepts
only version 1, loopback nonzero WebSocket URLs, and endpoints present in the
live PID/listener inventory; invalid files do not become candidates.

Modify Windows discovery to add marker candidates with source
`codex-app-bridge`. Update `select_windows_app_endpoint` to filter only that
source. Candidate deduplication gives bridge markers a higher source priority
than generic process discovery.

- [ ] **Step 4: Run focused tests and verify green**

Run: `cargo test app_marker` and `cargo test windows_app_target`.

Expected: all focused tests pass.

- [ ] **Step 5: Commit marker target work**

Run:

```text
git add src/app_bridge.rs src/lib.rs src/target.rs
git commit -m "fix: identify Windows Codex App through bridge markers"
```

### Task 2: Codex App stdio-to-WebSocket bridge

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/app_bridge.rs`
- Create: `src/bin/cdxm-codex-app-bridge.rs`
- Test: `src/app_bridge.rs`
- Test: `tests/app_bridge.rs`

- [ ] **Step 1: Add failing argument and executable-resolution tests**

Test an invocation with config arguments before `app-server`, an ordinary
`--version` passthrough, explicit `CDXM_REAL_CODEX`, Electron resources
fallback, per-user fallback, and self-recursion rejection.

```rust
assert_eq!(invocation_kind(&["-c".into(), "x=true".into(), "app-server".into()]), InvocationKind::AppServer { command_index: 2 });
assert_eq!(invocation_kind(&["--version".into()]), InvocationKind::Passthrough);
assert!(reject_recursive_executable(&bridge, &bridge).is_err());
```

- [ ] **Step 2: Run argument tests and verify red**

Run: `cargo test app_bridge::tests::invocation`, then
`cargo test app_bridge::tests::real_codex`.

Expected: compile failure because invocation classification and executable resolution are not implemented.

- [ ] **Step 3: Implement launch preparation**

Add `InvocationKind`, `RealCodexSources`, `invocation_kind`,
`resolve_real_codex`, loopback port allocation, readiness polling, and argument
rewriting. The rewritten command retains all arguments except an existing
`--listen` pair and appends exactly one bridge-owned `--listen` URL.

- [ ] **Step 4: Add a failing bidirectional proxy integration test**

Start a fake WebSocket server, feed initialize and thread/list JSON lines into
the bridge proxy, and assert exact frames in both directions, including a
server-initiated request and notification. Close stdin and assert marker
removal and child shutdown.

- [ ] **Step 5: Implement proxy runtime and binary**

Enable Tokio `io-std`. Implement:

```rust
pub async fn run_bridge(args: Vec<OsString>) -> anyhow::Result<i32>;
async fn proxy_jsonl_websocket(endpoint: &str) -> anyhow::Result<()>;
```

The proxy validates each nonempty stdin line as JSON, sends it as one text
frame, writes each text/binary JSON frame as one stdout line, handles ping and
close frames, and exits when stdin, WebSocket, or child server closes. Child
stdout is suppressed and child stderr is inherited so protocol stdout remains
clean. A marker guard removes its file on every normal/error exit.

- [ ] **Step 6: Run bridge tests and verify green**

Run: `cargo test app_bridge`.

Expected: unit and integration bridge tests pass.

- [ ] **Step 7: Commit bridge runtime**

Run:

```text
git add Cargo.toml Cargo.lock src/app_bridge.rs src/bin/cdxm-codex-app-bridge.rs tests/app_bridge.rs
git commit -m "feat: add Windows Codex App shared server bridge"
```

### Task 3: Reversible Windows installer integration

**Files:**
- Modify: `install.ps1`
- Modify: `tests/windows_installer_contract.rs`
- Modify: `README.md`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `skills/codex-monitor/references/codex-monitor-operations.md`

- [ ] **Step 1: Add failing installer contract tests**

Assert the installer exposes `-InstallAppBridge`, `-RemoveAppBridge`, and
`-RealCodexPath`; installs `cdxm-codex-app-bridge.exe`; saves prior
`CODEX_CLI_PATH` and `CDXM_REAL_CODEX`; does not overwrite the original backup
on idempotent enable; and restores only codex-monitor-owned values on removal.

- [ ] **Step 2: Run installer tests and verify red**

Run: `cargo test --test windows_installer_contract app_bridge`.

Expected: assertions fail because the flags and environment management are absent.

- [ ] **Step 3: Implement enable and removal functions**

Add mutually exclusive switches and these functions:

```powershell
function Resolve-RealCodexPath { param([string]$ExplicitPath) }
function Enable-CdxmAppBridge { param([string]$RealCodexPath) }
function Disable-CdxmAppBridge {}
```

Persist the original user environment values in
`$InstallRoot\app-bridge-env.json` before the first enable. Set
`CODEX_CLI_PATH` to `$BinDir\cdxm-codex-app-bridge.exe` and
copy the resolved executable to
`$InstallRoot\runtime\codex-app-real.exe` before setting `CDXM_REAL_CODEX` to
that managed copy. Removal restores the backup
only when the active `CODEX_CLI_PATH` still equals the installed bridge; an
unrelated later user override is preserved with a warning.

- [ ] **Step 4: Document activation, verification, and rollback**

Document:

```powershell
.\install.ps1 -Yes -NoShim -NoPath -InstallAppBridge -RealCodexPath <path>
.\install.ps1 -Yes -NoShim -NoPath -SkipBuild -RemoveAppBridge
```

State that App restart is required, `--target app` is marker-backed on
Windows, and acknowledgement without visible delivery is not sufficient.

- [ ] **Step 5: Run installer tests and verify green**

Run: `cargo test --test windows_installer_contract`.

Expected: all Windows installer contract tests pass.

- [ ] **Step 6: Commit installer and docs**

Run:

```text
git add install.ps1 tests/windows_installer_contract.rs README.md skills/codex-monitor/SKILL.md skills/codex-monitor/references/codex-monitor-operations.md
git commit -m "feat: install shared Codex App bridge on Windows"
```

### Task 4: Full verification and live acceptance

**Files:**
- Modify only if a verified defect is found in the preceding files.

- [ ] **Step 1: Run repository verification**

Run separately:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: every command exits zero with no failing test or warning.

- [ ] **Step 2: Install without restarting App**

Stop only processes locking installed codex-monitor binaries. Run the local
installer with explicit real Codex path and App bridge enablement. Restore any
stopped unrelated codex-monitor watcher with the same arguments.

```powershell
.\install.ps1 -Yes -NoShim -NoPath -InstallAppBridge -RealCodexPath "$HOME\AppData\Local\OpenAI\Codex\bin\codex.exe" -Source .
```

Expected: all three binaries and the skill install; user environment values
point to the bridge and real executable.

- [ ] **Step 3: Restart Codex App and re-open this thread**

Close and restart only Codex App after all code and installation checks pass.
The App must reopen the existing thread rather than start or fork one. Confirm
the bridge and real app-server processes are live and the marker source is
`codex-app-bridge`.

- [ ] **Step 4: Prove exact thread targeting**

Run:

```text
cdxm targets
cdxm --target app loaded
cdxm --target app threads --cwd C:\Users\ytvar\dev\codex-monitor
```

Expected: the visible thread id appears in loaded and cwd results. No generic
`codex-app-server-process` is accepted as `--target app`.

- [ ] **Step 5: Start the exact watcher and deliver a real message**

Apply `cdxm/codex` to the visible thread, then send a unique `agmsg` message to
`cdxm/codex` using the official scripts. Verify all three layers: app-server
ack, adapter state advancement, and visible message in this same App screen.

- [ ] **Step 6: Completion audit and final commit**

Review the full diff against the design, verify no temporary files/processes
remain, rerun relevant checks after any adjustment, and commit any final fixes.
