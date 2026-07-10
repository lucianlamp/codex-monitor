# Native App Monitor Shortcuts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep Windows Codex App native while providing `$codex-monitor` foreground wait, `$codex-monitor heartbeat`, and `$codex-monitor off`, and remove the unsigned App bridge from the product.

**Architecture:** A foreground Bash helper blocks inside the active App tool call and invokes only the installed agmsg `inbox.sh`, returning only when a message exists. The skill owns App shortcut orchestration and the Codex automation API, while the Rust release/updater and PowerShell installer shrink to the two public binaries and safely migrate only a proven-owned legacy bridge.

**Tech Stack:** Rust 2021, Bash/Git Bash, PowerShell 7/Windows PowerShell, Codex App heartbeat automations, agmsg scripts, Cargo integration tests.

---

### Task 1: Add the foreground agmsg wait helper

**Files:**
- Create: `skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh`
- Modify: `tests/installer_contract.rs`
- Modify: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Write failing foreground helper tests**

Add a Unix functional test that points `AGMSG_SCRIPTS_DIR` to a fake `inbox.sh`.
The fake script returns `No new messages.` twice and a message on the third call.
Assert that the helper exits zero, prints the message, and never prints the empty
result:

```rust
#[test]
fn foreground_helper_suppresses_empty_polls_and_returns_first_message() {
    let temp = tempfile::tempdir().unwrap();
    let scripts = temp.path().join("agmsg");
    fs::create_dir_all(&scripts).unwrap();
    let inbox = scripts.join("inbox.sh");
    fs::write(&inbox, r#"#!/usr/bin/env bash
count_file="$AGMSG_TEST_COUNT"
count=0
[[ -f "$count_file" ]] && count=$(cat "$count_file")
count=$((count + 1))
printf '%s\n' "$count" > "$count_file"
if (( count < 3 )); then
  printf 'No new messages.\n'
else
  printf '1 new message(s):\n\n  [now] sender: foreground-ready\n'
fi
"#).unwrap();
    fs::set_permissions(&inbox, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new("bash")
        .arg(repo_root().join("skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh"))
        .args(["cdxm", "codex"])
        .env("AGMSG_SCRIPTS_DIR", &scripts)
        .env("AGMSG_TEST_COUNT", temp.path().join("count"))
        .env("CDXM_FOREGROUND_POLL_SECONDS", "0")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foreground-ready"));
    assert!(!stdout.contains("No new messages."));
}
```

Add a platform-neutral contract test that requires the helper to call
`inbox.sh`, suppress `No new messages.`, avoid `nohup`/PID files/watch commands,
and exit after the first message.

- [ ] **Step 2: Run the focused tests and confirm failure**

Run:

```powershell
cargo test --test installer_contract foreground_helper_suppresses_empty_polls_and_returns_first_message
cargo test --test windows_installer_contract foreground_helper_contract
```

Expected: FAIL because `cdxm-agmsg-foreground.sh` does not exist.

- [ ] **Step 3: Implement the minimal blocking helper**

Create:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'usage: cdxm-agmsg-foreground.sh <team> <agent>\n' >&2
  exit 64
fi

team="$1"
agent="$2"
scripts_dir="${AGMSG_SCRIPTS_DIR:-$HOME/.agents/skills/agmsg/scripts}"
inbox="$scripts_dir/inbox.sh"
interval="${CDXM_FOREGROUND_POLL_SECONDS:-2}"

[[ -x "$inbox" ]] || {
  printf 'agmsg inbox script is missing or not executable: %s\n' "$inbox" >&2
  exit 69
}
[[ "$interval" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
  printf 'CDXM_FOREGROUND_POLL_SECONDS must be a non-negative number\n' >&2
  exit 64
}

while :; do
  output="$($inbox "$team" "$agent")"
  normalized="${output//$'\r'/}"
  if [[ -n "${normalized//[[:space:]]/}" && "$normalized" != "No new messages." ]]; then
    printf '%s\n' "$output"
    exit 0
  fi
  sleep "$interval"
done
```

- [ ] **Step 4: Run focused helper tests**

Run the two focused commands from Step 2.

Expected: PASS.

- [ ] **Step 5: Commit the helper slice**

```powershell
git add skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh tests/installer_contract.rs tests/windows_installer_contract.rs
git commit -m "feat: add foreground App inbox wait"
```

### Task 2: Rewrite the Codex App skill shortcut contract

**Files:**
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `skills/codex-monitor/references/codex-monitor-operations.md`
- Modify: `README.md`
- Modify: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Replace the stale bridge documentation test with three-mode assertions**

Require the skill and docs to contain:

```rust
for required in [
    "$codex-monitor`: foreground wait",
    "cdxm-agmsg-foreground.sh",
    "$codex-monitor heartbeat",
    "one-minute heartbeat",
    "automation_update",
    "$codex-monitor off",
    "target thread",
    "No new messages.",
] {
    assert!(skill.contains(required), "missing `{required}`");
}
for forbidden in [
    "-InstallAppBridge",
    "-RemoveAppBridge",
    "cdxm-codex-app-bridge.exe",
    "enable the app bridge",
] {
    assert!(!skill.contains(forbidden), "stale App bridge text `{forbidden}`");
}
```

Also assert that the heartbeat prompt says to use only installed agmsg scripts
and never manage watcher/process lifecycle.

- [ ] **Step 2: Run the contract test and confirm failure**

Run:

```powershell
cargo test --test windows_installer_contract docs_define_native_app_monitor_shortcuts
```

Expected: FAIL on stale App bridge instructions.

- [ ] **Step 3: Rewrite the App-specific skill section**

Keep current CLI behavior, but branch exact App shortcuts as follows:

```text
$codex-monitor
  resolve exact team/name
  run cdxm-agmsg-foreground.sh in the foreground
  present returned messages
  immediately re-enter wait

$codex-monitor heartbeat
  find matching automation by team/name/current thread
  use automation_update to create or update one active one-minute heartbeat
  prompt uses inbox.sh and send.sh only

$codex-monitor off
  treat the steer as cancellation of the owned foreground tool call
  delete only the matching current-thread heartbeat
  never stop a watcher or process
```

The heartbeat prompt must preserve the existing `agmsg monitor event` format.
Remove all recommendations to install, update, verify, target, or roll back the
unsigned App bridge. Document that Windows `--target app` is intentionally
unavailable and App delivery uses the shortcut modes.

- [ ] **Step 4: Update README and operations reference**

Replace the Windows App bridge section with native App instructions and the
three shortcut forms. State that `codex-monitor update` updates two public
binaries, preserves native/unowned `CODEX_CLI_PATH`, and migrates only a proven
legacy bridge.

- [ ] **Step 5: Run the documentation contract test**

Run the command from Step 2. Expected: PASS.

- [ ] **Step 6: Commit the skill contract**

```powershell
git add README.md skills/codex-monitor/SKILL.md skills/codex-monitor/references/codex-monitor-operations.md tests/windows_installer_contract.rs
git commit -m "docs: define native App monitor shortcuts"
```

### Task 3: Remove the unsigned App bridge binary and target path

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Modify: `src/target.rs`
- Modify: `.github/workflows/release.yml`
- Modify: `tests/cli_contract.rs`
- Modify: `tests/release_workflow_contract.rs`
- Delete: `src/bin/cdxm-codex-app-bridge.rs`
- Delete: `src/app_bridge.rs`
- Delete: `src/app_bridge/monitor_router.rs`
- Delete: `src/app_bridge/stdio_monitor.rs`
- Delete: `tests/app_bridge.rs`

- [ ] **Step 1: Write failing package and target tests**

Change release tests to require the Windows ZIP step to copy only
`codex-monitor.exe` and `cdxm.exe`, and explicitly reject
`cdxm-codex-app-bridge.exe`. Add a Cargo contract assertion that the manifest
contains no bridge binary target.

Replace Windows App target tests with:

```rust
#[test]
fn windows_app_target_directs_delivery_to_native_shortcuts() {
    let error = select_windows_app_endpoint(Vec::new()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("native Codex App"));
    assert!(message.contains("$codex-monitor"));
    assert!(message.contains("heartbeat"));
}
```

- [ ] **Step 2: Run focused tests and confirm failure**

Run:

```powershell
cargo test --test release_workflow_contract
cargo test --test cli_contract package_exposes_only_public_binaries
cargo test target::tests::windows_app_target_directs_delivery_to_native_shortcuts
```

Expected: FAIL while the bridge remains packaged and targetable.

- [ ] **Step 3: Delete bridge code and manifest entries**

Remove the `[[bin]] cdxm-codex-app-bridge` entry, remove
`pub mod app_bridge`, and delete the bridge implementation/tests listed above.
Keep WebSocket dependencies used by normal CLI transports.

- [ ] **Step 4: Make Windows App target fail safely**

Replace bridge candidate selection with one fixed error:

```rust
fn select_windows_app_endpoint(_: Vec<EndpointCandidate>) -> anyhow::Result<Endpoint> {
    bail!(
        "Windows uses the native Codex App runtime; use `$codex-monitor` for foreground delivery or `$codex-monitor heartbeat` for persistent delivery"
    )
}
```

Remove marker reads, bridge PID inventory parsing, bridge source priority, and
their unit tests. Preserve normal CLI endpoint discovery.

- [ ] **Step 5: Package only two Windows binaries**

Delete the bridge `Copy-Item` line from `.github/workflows/release.yml`.

- [ ] **Step 6: Run focused tests and commit**

Run the commands from Step 2, then:

```powershell
git add -A Cargo.toml src .github/workflows/release.yml tests
git commit -m "refactor: remove unsigned App bridge"
```

### Task 4: Shrink updater manifests and release archives to two binaries

**Files:**
- Modify: `src/update/model.rs`
- Modify: `src/update/archive.rs`
- Modify: `src/update/apply.rs`
- Modify: `src/update.rs`

- [ ] **Step 1: Change updater tests to the two-file model**

Require:

```rust
pub enum ManagedFile { CodexMonitor, Cdxm }
pub const ALL: [Self; 2] = [Self::CodexMonitor, Self::Cdxm];
pub const RELEASE: [Self; 2] = [Self::CodexMonitor, Self::Cdxm];
```

Update ZIP fixtures to two entries and assert `staged.len() == 2`. Remove tests
for optional App runtime companions. Keep duplicate, nested, unexpected,
checksum, transactional rollback, and locked-running-binary coverage.

- [ ] **Step 2: Run update unit tests and confirm failure**

Run:

```powershell
cargo test update::model::tests
cargo test update::archive::tests
cargo test update::apply::tests
```

Expected: FAIL because the current model requires seven files and three release
members.

- [ ] **Step 3: Implement the two-file model**

Delete App/runtime variants, runtime source names, optional-file logic, and
runtime staging extension. `UpdateManifest::validate_shape` must require exactly
the two fixed files with valid SHA-256 values.

In `run_update_windows`, download only the release ZIP:

```rust
let files = archive::download_latest_release(&release_base, &staging_root).await?;
```

Do not inspect or copy the Codex App package.

- [ ] **Step 4: Run update tests and commit**

Run the commands from Step 2, then:

```powershell
git add src/update.rs src/update/model.rs src/update/archive.rs src/update/apply.rs
git commit -m "refactor: reduce updater to public binaries"
```

### Task 5: Implement safe Windows legacy migration and simplify installer

**Files:**
- Modify: `src/update/windows.rs`
- Modify: `src/update.rs`
- Modify: `install.ps1`
- Modify: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Write failing migration and installer tests**

Add pure Rust tests for these cases:

```text
native CODEX_CLI_PATH + empty CDXM_REAL_CODEX -> preserve both
owned bridge + valid version-1 backup -> restore saved values
owned bridge + invalid/missing backup -> error without environment mutation
owned bridge exact executable path active -> error instructing App quit
fixed obsolete paths not active -> cleanup list contains bridge/runtime files
```

Change PowerShell contract tests to reject old bridge parameters and require
`Migrate-CdxmLegacyAppBridge`, saved environment fields, exact process
inventory, no `Stop-Process`, and a two-binary archive allowlist.

- [ ] **Step 2: Run focused tests and confirm failure**

Run:

```powershell
cargo test update::windows::tests
cargo test --test windows_installer_contract
```

Expected: FAIL on bridge installer options and runtime staging contracts.

- [ ] **Step 3: Simplify Rust Windows preflight/finalization**

Keep PATH normalization, update-result handling, parent PID waiting, and staging
cleanup. Remove AppX/runtime discovery. Expand `AppBridgeBackup` with saved
environment fields and compare normalized exact paths.

Before applying an update, reject only a running executable whose path equals a
fixed legacy bridge/runtime path. After applying:

```text
if current CODEX_CLI_PATH is the expected bridge:
  require valid matching ownership backup
  restore saved CODEX_CLI_PATH and CDXM_REAL_CODEX
else:
  preserve both current values
normalize public CLI PATH
remove fixed legacy files that are not in use
remove app-bridge-env.json only after successful owned migration
remove runtime directory only when empty
```

No function may stop a process.

- [ ] **Step 4: Simplify PowerShell installer**

Remove bridge parameters, AppX discovery, runtime copying, bridge enabling, and
bridge removal modes. Add `Migrate-CdxmLegacyAppBridge` before binary install.
It uses the same ownership and exact-path rules as Rust, updates environment
only when ownership is proven, removes fixed obsolete files only when unused,
and preserves native explicit `CODEX_CLI_PATH`.

Change the prebuilt allowlist and expected-file check to exactly:

```powershell
$allowed = @('codex-monitor.exe', 'cdxm.exe')
```

- [ ] **Step 5: Run focused tests and commit**

Run the commands from Step 2, then:

```powershell
git add src/update.rs src/update/windows.rs install.ps1 tests/windows_installer_contract.rs
git commit -m "fix: migrate Windows App back to native runtime"
```

### Task 6: Full verification, installation, and live acceptance

**Files:**
- Modify only if verification exposes a defect.
- Install from: current feature worktree
- Install to: `C:\Users\ytvar\.codex-monitor` and `C:\Users\ytvar\.codex\skills\codex-monitor`

- [ ] **Step 1: Run static and automated verification**

Run:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: all pass.

- [ ] **Step 2: Review the final diff for scope and dead files**

Run:

```powershell
git status --short
git diff --stat HEAD~5
git diff HEAD~5 -- Cargo.toml install.ps1 src skills tests README.md .github/workflows/release.yml
rg -n "cdxm-codex-app-bridge|InstallAppBridge|RemoveAppBridge|codex-app-real|codex-code-mode-host" Cargo.toml install.ps1 src skills tests README.md .github/workflows/release.yml
```

Expected: no active product reference to the removed bridge/runtime; historical
specs/plans may retain evidence.

- [ ] **Step 3: Install from the worktree without changing the CLI shim**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Yes -Source . -NoShim -BuildFromSource
```

Expected: two binaries and the skill install; explicit native
`CODEX_CLI_PATH` remains unchanged; `CDXM_REAL_CODEX` remains empty; installed
bridge/runtime files are absent.

- [ ] **Step 4: Prove foreground delivery**

Send one unique self-addressed event through
`~/.agents/skills/agmsg/scripts/send.sh`, then run the installed
`cdxm-agmsg-foreground.sh cdxm codex` through Git Bash. Present the returned
event and verify the helper exits without printing an empty poll. Do not start
or stop a watcher.

- [ ] **Step 5: Prove heartbeat and off**

Use the exact skill contract and Codex automation API to upsert one matching
one-minute heartbeat. Send one unique self-addressed event, verify it appears in
this task, then invoke the off path and verify the matching automation is
deleted. Do not touch any watcher/process lifecycle.

- [ ] **Step 6: Reconfirm App, Browser, and CLI independence**

Verify native App process identity, `Get-Command codex -All`,
`Get-Command cdxm`, `codex --version`, and `cdxm --version`. Using the Browser
skill, load `https://www.google.com/`, require title `Google`, and require a
non-empty DOM snapshot.

- [ ] **Step 7: Commit any verification fixes and finish the branch**

If no fixes were needed, keep the worktree clean. Then use the
finishing-a-development-branch skill to present the branch handoff options.
