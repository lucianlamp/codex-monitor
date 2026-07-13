# Codex App Stop Hook Monitor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Codex App `$codex-monitor` arm a session-scoped Stop hook that waits for agmsg without empty model turns, while leaving Codex CLI durable watchers unchanged.

**Architecture:** Add a focused Rust `app_hook` module for global hook merging, atomic session markers, Stop payload handling, and foreground agmsg output formatting. Expose idempotent `app-hook enable|disable|status` commands plus a hidden `__app-stop-hook` handler, then route only the App skill shortcuts to them. The handler uses the installed agmsg inbox path and never manages watcher processes.

**Tech Stack:** Rust 2021, Clap, Serde/serde_json, Tokio process execution, Bash/Git Bash, Codex Stop hooks, Cargo integration tests.

---

### Task 1: Hook configuration and session markers

**Files:**
- Create: `src/app_hook.rs`
- Modify: `src/lib.rs`
- Test: `src/app_hook.rs`

- [ ] **Step 1: Write failing unit tests for hook merge and marker scope**

Add tests covering preservation of unrelated hooks, idempotent installation,
replacement of only the codex-monitor handler, invalid JSON refusal, atomic
marker enable/status/disable, invalid session ids, and cwd mismatch. Use an
injected `AppHookPaths` rooted in `tempfile::TempDir`.

```rust
#[test]
fn install_preserves_other_hooks_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppHookPaths::for_test(temp.path());
    fs::write(&paths.hooks_json, r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}}"#).unwrap();
    assert_eq!(ensure_hook_installed(&paths, Path::new("/opt/codex-monitor")).unwrap(), HookChange::Added);
    let once = fs::read(&paths.hooks_json).unwrap();
    assert_eq!(ensure_hook_installed(&paths, Path::new("/opt/codex-monitor")).unwrap(), HookChange::Unchanged);
    assert_eq!(fs::read(&paths.hooks_json).unwrap(), once);
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test app_hook::tests -- --nocapture`

Expected: compilation fails because `app_hook` does not exist.

- [ ] **Step 3: Implement hook models, semantic merge, and atomic markers**

Create these public boundaries:

```rust
pub struct AppHookPaths { pub hooks_json: PathBuf, pub markers_dir: PathBuf }
pub enum HookChange { Added, Updated, Unchanged }
pub struct AppHookMarker { pub version: u32, pub session_id: String, pub team: String, pub name: String, pub cwd: PathBuf, pub updated_at: String }
pub fn default_paths() -> anyhow::Result<AppHookPaths>;
pub fn ensure_hook_installed(paths: &AppHookPaths, executable: &Path) -> anyhow::Result<HookChange>;
pub fn enable_marker(paths: &AppHookPaths, marker: &AppHookMarker) -> anyhow::Result<()>;
pub fn load_marker(paths: &AppHookPaths, session_id: &str) -> anyhow::Result<Option<AppHookMarker>>;
pub fn disable_marker(paths: &AppHookPaths, session_id: &str) -> anyhow::Result<bool>;
```

Use `~/.codex/hooks.json`, `~/.codex-monitor/app-hooks/<session>.json`, a
constant status message to identify only the owned handler, `timeout: 86400`,
and same-directory temporary writes followed by rename. Do not rewrite an
unchanged hooks file.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run: `cargo test app_hook::tests -- --nocapture`

Expected: all `app_hook` tests pass.

- [ ] **Step 5: Commit the configuration slice**

```bash
git add src/app_hook.rs src/lib.rs
git commit -m "feat: add App Stop hook state"
```

### Task 2: CLI management surface

**Files:**
- Modify: `src/cli.rs`
- Test: `tests/cli_contract.rs`

- [ ] **Step 1: Write failing CLI contract tests**

Add tests asserting that top-level help exposes `app-hook`, nested help exposes
`enable`, `disable`, and `status`, and top-level help hides `__app-stop-hook`.

```rust
#[test]
fn app_hook_commands_are_public_and_handler_is_hidden() {
    let help = command_output(["--help"]);
    assert!(help.contains("app-hook"));
    assert!(!help.contains("__app-stop-hook"));
    let nested = command_output(["app-hook", "--help"]);
    for name in ["enable", "disable", "status"] { assert!(nested.contains(name)); }
}
```

- [ ] **Step 2: Run the CLI test and verify RED**

Run: `cargo test --test cli_contract app_hook_commands_are_public_and_handler_is_hidden -- --exact`

Expected: FAIL because `app-hook` is absent.

- [ ] **Step 3: Add Clap commands and dispatch**

Add `Commands::AppHook { command: AppHookCommand }` and hidden
`Commands::AppStopHook`. `enable` requires team/name/session/cwd, calls
`ensure_hook_installed` using `current_exe()`, writes the marker, and reports
whether `/hooks` review is required. `disable` removes only the exact session
marker. `status` prints machine-readable tab-separated installed/enabled/team/
name/cwd fields.

- [ ] **Step 4: Run CLI and app_hook tests**

Run: `cargo test --test cli_contract app_hook_commands_are_public_and_handler_is_hidden -- --exact && cargo test app_hook::tests`

Expected: both commands pass.

- [ ] **Step 5: Commit the CLI slice**

```bash
git add src/cli.rs tests/cli_contract.rs
git commit -m "feat: manage App Stop hook sessions"
```

### Task 3: Stop hook execution and message continuation

**Files:**
- Modify: `src/app_hook.rs`
- Modify: `src/cli.rs`
- Modify: `skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh`
- Test: `src/app_hook.rs`
- Test: `tests/installer_contract.rs`
- Test: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Write failing handler tests**

Use a fake foreground helper selected through `CDXM_APP_HOOK_FOREGROUND_HELPER`
and test marker absence, cwd mismatch, `stop_hook_active` false and true,
multiple inbox rows, JSON escaping, helper failure, and parent-loss cleanup.

```rust
#[tokio::test]
async fn active_marker_returns_a_stop_continuation() {
    let output = run_stop_hook_with_paths(&paths, input, &helper).await.unwrap();
    assert_eq!(output["decision"], "block");
    assert!(output["reason"].as_str().unwrap().contains("agmsg monitor event"));
}
```

- [ ] **Step 2: Run the handler tests and verify RED**

Run: `cargo test app_hook::tests::stop_hook -- --nocapture`

Expected: FAIL because Stop payload execution is absent.

- [ ] **Step 3: Implement Stop input handling and owned helper execution**

Deserialize the official fields:

```rust
#[derive(Deserialize)]
struct StopHookInput {
    session_id: String,
    cwd: PathBuf,
    turn_id: String,
    stop_hook_active: bool,
}
```

For an inactive marker, emit `{"continue":true}` immediately. For an active
marker, run the installed foreground helper with captured stdout/stderr and
`CDXM_FOREGROUND_PARENT_PID` set to the handler PID. Parse each stable inbox
row into one `agmsg monitor event` block and return
`{"decision":"block","reason":"..."}`. Propagate helper failure as a nonzero
handler result and keep stdout JSON-only.

- [ ] **Step 4: Make the foreground helper exit when its owner disappears**

Capture `CDXM_FOREGROUND_PARENT_PID` once and check it before every poll. On
Git Bash/Unix, `kill -0` failure exits without reading or printing a message.
Keep the existing behavior unchanged when the variable is absent.

- [ ] **Step 5: Run handler and installer contract tests**

Run: `cargo test app_hook::tests && cargo test --test installer_contract foreground_helper && cargo test --test windows_installer_contract foreground_helper_contract`

Expected: all focused tests pass and no detached lifecycle behavior is added.

- [ ] **Step 6: Commit the execution slice**

```bash
git add src/app_hook.rs src/cli.rs skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh tests/installer_contract.rs tests/windows_installer_contract.rs
git commit -m "feat: continue App turns from Stop hooks"
```

### Task 4: Route App shortcuts and preserve CLI behavior

**Files:**
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `skills/codex-monitor/references/codex-monitor-operations.md`
- Modify: `README.md`
- Modify: `src/target.rs`
- Test: `tests/windows_installer_contract.rs`
- Test: `tests/cli_contract.rs`

- [ ] **Step 1: Write failing routing contract tests**

Require App `$codex-monitor` to call `app-hook enable` with
`CODEX_THREAD_ID`, require App off to call `app-hook disable`, keep heartbeat as
fallback, forbid the foreground helper as the App-visible default, and retain
the existing CLI `cdxm-agmsg-apply.sh` durable path.

- [ ] **Step 2: Run routing tests and verify RED**

Run: `cargo test --test windows_installer_contract`

Expected: FAIL because the skill still defines direct foreground wait.

- [ ] **Step 3: Update skill, operations, README, and target diagnostics**

Document the exact App commands, one-time `/hooks` trust, interrupt-then-off
workflow, 24-hour timeout, heartbeat fallback, and unchanged CLI watcher/
LaunchAgent semantics. Replace the Windows App target error text so it points
to Stop-hook `$codex-monitor` rather than foreground delivery.

- [ ] **Step 4: Run routing and CLI tests**

Run: `cargo test --test windows_installer_contract && cargo test --test cli_contract && cargo test target::tests::windows_app_target_directs_delivery_to_native_shortcuts`

Expected: all routing contracts pass.

- [ ] **Step 5: Commit the routing slice**

```bash
git add skills/codex-monitor/SKILL.md skills/codex-monitor/references/codex-monitor-operations.md README.md src/target.rs tests/windows_installer_contract.rs tests/cli_contract.rs
git commit -m "docs: route App monitor through Stop hooks"
```

### Task 5: Install, verify, and ship

**Files:**
- Test: `tests/installer_contract.rs`
- Test: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Add installer assertions for complete skill delivery**

Verify both installers copy the updated skill and foreground helper, while no
installer enables a session marker, trusts a hook, or starts a watcher.

- [ ] **Step 2: Run the full repository verification**

Run:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
bash -n skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh
git diff --check
```

Expected: all commands succeed.

- [ ] **Step 3: Test with temporary HOME and fake agmsg scripts**

Enable a temporary session, inspect the merged hook JSON and marker, feed a
Stop payload to `__app-stop-hook`, assert one continuation JSON object, repeat
with `stop_hook_active: true`, then disable and assert immediate no-op. Use no
real watcher process.

- [ ] **Step 4: Install the verified binary and skill locally**

Run the source installer without changing the Codex CLI shim. Confirm the
installed skill matches the repository and `codex-monitor app-hook --help`
shows the new commands.

- [ ] **Step 5: Apply the dormant hook and current-session marker**

Run `app-hook enable` for the current App `CODEX_THREAD_ID`, team, name, and
cwd. Inspect `~/.codex/hooks.json` to prove unrelated hooks were preserved. Do
not edit `[hooks.state]`; report the required `/hooks` trust action if Codex
marks the handler untrusted.

- [ ] **Step 6: Live acceptance after trust**

Send one unique self-addressed agmsg message, allow the current turn to stop,
verify the visible continuation, then send a second message to prove automatic
re-arm. Interrupt, run `$codex-monitor off`, and verify the marker is removed.
No watcher process may be started or stopped.

- [ ] **Step 7: Commit any installer test changes**

```bash
git add tests/installer_contract.rs tests/windows_installer_contract.rs
git commit -m "test: verify App Stop hook installation"
```

- [ ] **Step 8: Integrate and publish**

Fast-forward the verified branch into `main`, push `main`, verify the remote
SHA, then remove only this clean worktree and delete `codex/app-stop-hook`.
