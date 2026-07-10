# `codex-monitor update` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Windows `codex-monitor update` command that securely updates all installed codex-monitor executables and refreshes the private Codex App runtime in one rollback-safe operation.

**Architecture:** The visible command performs read-only preflight, downloads and validates a fixed release archive, stages the matching Codex App runtime, and launches a temporary copy of itself as a hidden apply worker. The worker waits for the caller to exit, validates a fixed-file manifest, applies same-volume replacements transactionally, restores backups on failure, reasserts the owned user environment, and persists a result for the next invocation.

**Tech Stack:** Rust 2021, clap, Tokio, reqwest with rustls, sha2, serde/serde_json, zip, windows-sys, PowerShell only for AppX package metadata and user-scoped environment values, PowerShell installer contract tests, GitHub Actions release packaging.

---

## File Structure

- Create `src/update.rs`: public command entrypoints, orchestration, helper spawn, prior-result reporting, and non-Windows rejection.
- Create `src/update/model.rs`: fixed managed-file identifiers, manifest/result schemas, install paths, and hash helpers.
- Create `src/update/archive.rs`: checksum parsing, release download, strict ZIP validation, and release staging.
- Create `src/update/apply.rs`: destination preparation, transactional replacement, verification, rollback, and result persistence.
- Create `src/update/windows.rs`: Windows process preflight, AppX runtime discovery, parent-process wait, and owned environment handling.
- Modify `src/lib.rs`: export the updater module.
- Modify `src/cli.rs`: expose `update`, hide the internal apply command, dispatch both paths, and surface prior helper failures.
- Modify `Cargo.toml` and `Cargo.lock`: add Windows updater dependencies.
- Modify `.github/workflows/release.yml`: package the App bridge in the Windows release ZIP.
- Modify `install.ps1`: require and extract the same three release executables.
- Modify `tests/cli_contract.rs`: prove the public and hidden CLI surfaces.
- Modify `tests/release_workflow_contract.rs`: prove Windows packaging includes the bridge.
- Modify `tests/windows_installer_contract.rs`: prove prebuilt installation requires the bridge.
- Modify `README.md`: replace the checkout-based update instruction with `codex-monitor update`.
- Modify `skills/codex-monitor/SKILL.md`: teach the installed skill the update preconditions and command.

### Task 1: CLI Contract and Updater Boundary

**Files:**
- Modify: `tests/cli_contract.rs`
- Modify: `src/cli.rs:36-69,314-460`
- Modify: `src/lib.rs:1-15`
- Create: `src/update.rs`

- [ ] **Step 1: Write the failing CLI contract tests**

Add tests that require the public command and keep the worker private:

```rust
#[test]
fn update_command_is_public_and_apply_worker_is_hidden() {
    let primary = env!("CARGO_BIN_EXE_codex-monitor");
    let help = Command::new(primary).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("update"));
    assert!(!help.contains("__apply-update"));

    let update = Command::new(primary).args(["update", "--help"]).output().unwrap();
    assert!(update.status.success());
}
```

- [ ] **Step 2: Run the contract test and verify it fails**

Run: `cargo test --test cli_contract update_command_is_public_and_apply_worker_is_hidden -- --exact`

Expected: FAIL because `update` is not a recognized subcommand.

- [ ] **Step 3: Add the command variants and module entrypoints**

Add these variants to `Commands`:

```rust
Update,
#[command(name = "__apply-update", hide = true)]
ApplyUpdate {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    parent_pid: u32,
},
```

Create `src/update.rs` with compile-safe entrypoints:

```rust
use anyhow::Result;
use std::path::Path;

#[cfg(windows)]
pub async fn run_update() -> Result<i32> {
    windows_impl::run_update().await
}

#[cfg(not(windows))]
pub async fn run_update() -> Result<i32> {
    anyhow::bail!("codex-monitor update with Codex App runtime refresh is currently Windows-only")
}

#[cfg(windows)]
pub fn run_apply(manifest: &Path, parent_pid: u32) -> Result<i32> {
    windows_impl::run_apply(manifest, parent_pid)
}

#[cfg(not(windows))]
pub fn run_apply(_manifest: &Path, _parent_pid: u32) -> Result<i32> {
    anyhow::bail!("the internal update worker is currently Windows-only")
}

pub fn report_previous_failure() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
mod windows_impl {
    use anyhow::Result;
    use std::path::Path;

    pub async fn run_update() -> Result<i32> {
        anyhow::bail!("update staging is not implemented")
    }

    pub fn run_apply(_manifest: &Path, _parent_pid: u32) -> Result<i32> {
        anyhow::bail!("update apply is not implemented")
    }
}
```

Export `pub mod update;` from `src/lib.rs`, call
`crate::update::report_previous_failure()?` before parsing normal arguments,
and dispatch the variants:

```rust
Commands::Update => crate::update::run_update().await,
Commands::ApplyUpdate { manifest, parent_pid } => {
    crate::update::run_apply(&manifest, parent_pid)
}
```

- [ ] **Step 4: Run the contract test and verify it passes**

Run: `cargo test --test cli_contract update_command_is_public_and_apply_worker_is_hidden -- --exact`

Expected: PASS.

- [ ] **Step 5: Commit the CLI boundary**

```text
git add src/cli.rs src/lib.rs src/update.rs tests/cli_contract.rs
git commit -m "feat: add codex-monitor update command"
```

### Task 2: Release and Installer Binary Set

**Files:**
- Modify: `tests/release_workflow_contract.rs`
- Modify: `tests/windows_installer_contract.rs`
- Modify: `.github/workflows/release.yml:48-56`
- Modify: `install.ps1:77-149`

- [ ] **Step 1: Write failing packaging contract assertions**

Require both release and installer paths to name the bridge:

```rust
#[test]
fn windows_release_packages_app_bridge() {
    let wf = workflow();
    assert!(wf.contains("release/cdxm-codex-app-bridge.exe"));
    assert!(wf.contains("Copy-Item \"target/${{ matrix.target }}/release/cdxm-codex-app-bridge.exe\""));
}
```

Extend `windows_installer_has_prebuilt_download_path` with:

```rust
assert!(installer.contains("$allowed = @('codex-monitor.exe', 'cdxm.exe', 'cdxm-codex-app-bridge.exe')"));
assert!(installer.contains("Join-Path $BinDir 'cdxm-codex-app-bridge.exe'"));
```

- [ ] **Step 2: Run the packaging tests and verify they fail**

Run: `cargo test --test release_workflow_contract --test windows_installer_contract`

Expected: FAIL because the current ZIP and prebuilt allowlist contain only two executables.

- [ ] **Step 3: Package and validate all three Windows executables**

In `.github/workflows/release.yml`, add:

```powershell
Copy-Item "target/${{ matrix.target }}/release/cdxm-codex-app-bridge.exe" $staging
```

In `Install-CdxmPrebuilt`, use:

```powershell
$allowed = @('codex-monitor.exe', 'cdxm.exe', 'cdxm-codex-app-bridge.exe')
```

After extraction, fail unless all three files exist. Preserve mandatory checksum
validation and exact top-level entry matching.

- [ ] **Step 4: Run the packaging tests and verify they pass**

Run: `cargo test --test release_workflow_contract --test windows_installer_contract`

Expected: PASS.

- [ ] **Step 5: Commit packaging consistency**

```text
git add .github/workflows/release.yml install.ps1 tests/release_workflow_contract.rs tests/windows_installer_contract.rs
git commit -m "fix: ship app bridge in Windows releases"
```

### Task 3: Fixed Manifest, Hashing, and Strict Release Staging

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/update.rs`
- Create: `src/update/model.rs`
- Create: `src/update/archive.rs`

- [ ] **Step 1: Add failing model and archive tests**

Define tests beside the new modules that prove:

```rust
#[test]
fn managed_files_map_only_to_fixed_destinations() {
    let root = Path::new(r"C:\Users\me\.codex-monitor");
    assert_eq!(ManagedFile::CodexMonitor.destination(root), root.join("bin/codex-monitor.exe"));
    assert_eq!(ManagedFile::RealCodex.destination(root), root.join("runtime/codex-app-real.exe"));
}

#[test]
fn checksum_requires_one_lower_or_upper_hex_digest() {
    assert_eq!(parse_checksum(&"a".repeat(64)).unwrap(), "a".repeat(64));
    assert!(parse_checksum("missing").is_err());
    assert!(parse_checksum(&("a".repeat(64) + " extra")).is_err());
}

#[test]
fn release_zip_rejects_missing_duplicate_nested_and_unexpected_members() {
    // Build in-memory ZIP fixtures and assert each invalid layout returns Err.
}
```

- [ ] **Step 2: Run updater unit tests and verify they fail to compile**

Run: `cargo test update::model update::archive`

Expected: FAIL because `ManagedFile`, `parse_checksum`, and strict extraction do not exist.

- [ ] **Step 3: Add updater dependencies and fixed schemas**

Add `zip` as a normal dependency so archive validation tests run on every CI
platform. Add `reqwest` and `windows-sys` under the Windows target dependency
section. Keep the existing non-Windows reqwest entry for remote control.

Define fixed identifiers instead of serializing arbitrary destination paths:

```rust
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedFile {
    CodexMonitor,
    Cdxm,
    AppBridge,
    RealCodex,
    CodeModeHost,
    CommandRunner,
    SandboxSetup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedFile {
    pub id: ManagedFile,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: u32,
    pub install_root: PathBuf,
    pub staging_root: PathBuf,
    pub files: Vec<StagedFile>,
}
```

`ManagedFile::destination`, `ManagedFile::staged_name`, and apply ordering must
be exhaustive matches. `None` hashes are valid only for optional runtime files
and mean that an obsolete destination is removed transactionally.

- [ ] **Step 4: Implement checksum and archive validation**

Implement these boundaries in `src/update/archive.rs`:

```rust
pub const WINDOWS_ARCHIVE: &str = "codex-monitor-x86_64-pc-windows-msvc.zip";

pub fn parse_checksum(text: &str) -> anyhow::Result<String>;
pub fn verify_sha256(bytes: &[u8], expected: &str) -> anyhow::Result<()>;
pub fn extract_release_zip(bytes: &[u8], destination: &Path) -> anyhow::Result<Vec<StagedFile>>;

#[cfg(windows)]
pub async fn download_latest_release(base: &str, destination: &Path)
    -> anyhow::Result<Vec<StagedFile>>;
```

Iterate every ZIP entry, require a top-level UTF-8 name in the exact set
`codex-monitor.exe`, `cdxm.exe`, and `cdxm-codex-app-bridge.exe`, reject
directories and duplicates, stream each accepted entry to a newly created
destination, then verify all required identifiers were seen.

- [ ] **Step 5: Run model and archive tests**

Run: `cargo test update::model update::archive`

Expected: PASS.

- [ ] **Step 6: Commit secure staging primitives**

```text
git add Cargo.toml Cargo.lock src/update.rs src/update/model.rs src/update/archive.rs
git commit -m "feat: validate codex-monitor update payloads"
```

### Task 4: Windows Preflight and App Runtime Discovery

**Files:**
- Modify: `src/update.rs`
- Create: `src/update/windows.rs`

- [ ] **Step 1: Write failing pure preflight tests**

Cover owned bridge paths, process blocking, and AppX metadata parsing:

```rust
#[test]
fn app_process_inventory_blocks_update_case_insensitively() {
    assert!(blocks_update_process("Codex.exe"));
    assert!(blocks_update_process("CDXM-CODEX-APP-BRIDGE.EXE"));
    assert!(blocks_update_process("codex-app-real.exe"));
    assert!(!blocks_update_process("codex-monitor.exe"));
}

#[test]
fn ownership_requires_backup_and_matching_user_bridge() {
    let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
    let backup = AppBridgeBackup { version: 1, bridge_path: paths.app_bridge.clone() };
    assert!(verify_owned_bridge(&paths, &backup, &paths.app_bridge).is_ok());
    assert!(verify_owned_bridge(&paths, &backup, Path::new(r"C:\other.exe")).is_err());
}
```

- [ ] **Step 2: Run the Windows updater tests and verify they fail**

Run: `cargo test update::windows`

Expected: FAIL because the Windows preflight module does not exist.

- [ ] **Step 3: Implement AppX and environment inventory**

Resolve the absolute system PowerShell path and run one non-interactive command
that returns compact JSON containing:

```powershell
[ordered]@{
  installLocation = (Get-AppxPackage -Name 'OpenAI.Codex' |
    Sort-Object Version -Descending | Select-Object -First 1).InstallLocation
  userCodexCliPath = [Environment]::GetEnvironmentVariable('CODEX_CLI_PATH', 'User')
  userRealCodex = [Environment]::GetEnvironmentVariable('CDXM_REAL_CODEX', 'User')
} | ConvertTo-Json -Compress
```

Resolve `<installLocation>\app\resources\codex.exe`, require its sibling
code-mode host, record optional siblings, and reject a real executable that
resolves to the installed bridge.

- [ ] **Step 4: Implement running-process and ownership preflight**

Use Toolhelp process enumeration from `windows-sys` and reject process names
`Codex.exe`, `cdxm-codex-app-bridge.exe`, `codex-app-real.exe`, and the managed
runtime helper names. Read `app-bridge-env.json`, require version 1 and a
`bridgePath` matching the install root, then require the user-scoped
`CODEX_CLI_PATH` to match the same path.

Expose:

```rust
pub struct WindowsPreflight {
    pub paths: InstallPaths,
    pub runtime_sources: RuntimeSources,
}

pub fn preflight() -> anyhow::Result<WindowsPreflight>;
pub fn stage_runtime(preflight: &WindowsPreflight, staging: &Path)
    -> anyhow::Result<Vec<StagedFile>>;
pub fn wait_for_process_exit(pid: u32) -> anyhow::Result<()>;
pub fn reassert_owned_environment(paths: &InstallPaths) -> anyhow::Result<()>;
```

- [ ] **Step 5: Run Windows preflight tests**

Run: `cargo test update::windows`

Expected: PASS.

- [ ] **Step 6: Commit Windows discovery and preflight**

```text
git add src/update.rs src/update/windows.rs
git commit -m "feat: preflight Windows Codex App updates"
```

### Task 5: Transactional Apply and Rollback

**Files:**
- Modify: `src/update.rs`
- Modify: `src/update/model.rs`
- Create: `src/update/apply.rs`

- [ ] **Step 1: Write failing transactional tests**

Use temporary install and staging roots for three cases:

```rust
#[test]
fn apply_installs_complete_manifest_and_removes_absent_optional_files() {
    // Seed old destinations, stage new required files, mark CommandRunner absent,
    // apply, and assert every required hash plus removal of the stale runner.
}

#[test]
fn apply_skips_identical_files_without_leaving_backups() {
    // Put identical bytes at source and destination, apply, and assert no
    // backup or destination-temporary files remain.
}

#[test]
fn apply_failure_restores_every_destination() {
    // Inject a failure before the third replacement and assert every original
    // byte sequence is restored and every new temporary is removed.
}
```

- [ ] **Step 2: Run apply tests and verify they fail to compile**

Run: `cargo test update::apply`

Expected: FAIL because transactional apply does not exist.

- [ ] **Step 3: Implement prepare, apply, verify, and rollback**

Expose one production entrypoint:

```rust
pub fn apply_manifest(manifest: &UpdateManifest) -> anyhow::Result<ApplySummary>;
```

Implementation order:

1. Validate manifest version, install root, unique identifiers, required
   identifiers, and every staged hash.
2. Copy changed staged files to destination-directory temporary files and
   verify those hashes.
3. Rename existing destinations into one install-root backup directory.
4. Rename prepared files into place in bridge/runtime, `cdxm`, then
   `codex-monitor` order; absent optional files remain backed up and uninstalled.
5. Verify every final state.
6. Remove backups and temporaries on success.
7. On any error, remove installed new files and restore backups in reverse order.

Use a private `apply_manifest_with_hook` in tests to inject a deterministic
failure before a selected identifier without introducing production-only fault
flags.

- [ ] **Step 4: Implement atomic result persistence and reporting**

Add:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateResult {
    pub version: u32,
    pub success: bool,
    pub message: String,
}

pub fn write_result_atomic(path: &Path, result: &UpdateResult) -> anyhow::Result<()>;
pub fn take_previous_failure(path: &Path) -> anyhow::Result<Option<String>>;
```

Success results are silently consumed. Failure results return their message
exactly once and are removed only after a successful read.

- [ ] **Step 5: Run apply and result tests**

Run: `cargo test update::apply update::model`

Expected: PASS.

- [ ] **Step 6: Commit transaction support**

```text
git add src/update.rs src/update/model.rs src/update/apply.rs
git commit -m "feat: apply codex-monitor updates transactionally"
```

### Task 6: End-to-End Orchestration and Helper Handoff

**Files:**
- Modify: `src/update.rs`
- Modify: `src/update/model.rs`
- Modify: `src/update/windows.rs`
- Modify: `src/cli.rs`

- [ ] **Step 1: Write failing orchestration tests**

Factor deterministic builders and test that:

```rust
#[test]
fn complete_manifest_requires_all_binary_and_runtime_ids() {
    let manifest = manifest_fixture();
    assert!(manifest.validate().is_ok());
    let incomplete = manifest.without(ManagedFile::AppBridge);
    assert!(incomplete.validate().is_err());
}

#[test]
fn helper_arguments_use_absolute_manifest_and_current_parent_pid() {
    let args = helper_args(Path::new(r"C:\tmp\manifest.json"), 42);
    assert_eq!(args, ["__apply-update", "--manifest", r"C:\tmp\manifest.json", "--parent-pid", "42"]);
}
```

- [ ] **Step 2: Run orchestration tests and verify they fail**

Run: `cargo test update::tests`

Expected: FAIL because manifest composition and helper argument construction are absent.

- [ ] **Step 3: Implement visible update orchestration**

`run_update` must:

1. Run Windows preflight before downloading or mutating.
2. Create a unique staging directory below the install root so helper and
   result paths remain on a trusted volume.
3. Download, checksum, and strictly extract the release.
4. Stage the resolved App runtime and compose one fixed-file manifest.
5. Serialize the manifest atomically.
6. Copy `std::env::current_exe()` to `update-helper.exe` in staging.
7. Spawn the helper with inherited standard streams and the hidden command.
8. Print that the verified update has been handed off and return success only
   after spawn succeeds.
9. Remove staging on every pre-handoff failure.

- [ ] **Step 4: Implement hidden apply orchestration**

`run_apply` must wait for the parent PID, reread and validate the manifest,
rerun App-process preflight, apply the transaction, reassert owned user
environment values, write the result, print the final status, and clean staging
on success. On failure, it writes the failure result before returning nonzero.

Wire `report_previous_failure` to the fixed result path before normal CLI parse.
The reported message begins with `Previous codex-monitor update failed:`.

- [ ] **Step 5: Run all updater tests and CLI contracts**

Run: `cargo test update:: --lib`

Expected: PASS.

Run: `cargo test --test cli_contract`

Expected: PASS.

- [ ] **Step 6: Commit the complete command flow**

```text
git add src/update.rs src/update/model.rs src/update/windows.rs src/cli.rs
git commit -m "feat: complete Windows codex-monitor updater"
```

### Task 7: Documentation and Operator Contract

**Files:**
- Modify: `README.md:103-129`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Extend the documentation contract test**

Require README to contain the actual command and shutdown precondition:

```rust
assert!(readme.contains("codex-monitor update"));
assert!(readme.contains("fully quit Codex App"));
```

- [ ] **Step 2: Run the documentation contract and verify it fails**

Run: `cargo test --test windows_installer_contract readme_documents_windows_native_install -- --exact`

Expected: FAIL because README still tells users to rerun the installer.

- [ ] **Step 3: Document the final update workflow**

Replace the rerun-installer sentence with:

```powershell
# Fully quit Codex App first, then run from any directory.
codex-monitor update
```

Explain that the command verifies the latest release, updates all three
codex-monitor executables, refreshes the App-bundled runtime, preserves bridge
ownership, and tells the user to reopen Codex App. State that it refuses to run
while App or the bridge is active.

Add the same operational rule to `skills/codex-monitor/SKILL.md` without making
the command manage watcher lifecycle.

- [ ] **Step 4: Run documentation and installer contracts**

Run: `cargo test --test windows_installer_contract --test release_workflow_contract`

Expected: PASS.

- [ ] **Step 5: Commit documentation**

```text
git add README.md skills/codex-monitor/SKILL.md tests/windows_installer_contract.rs
git commit -m "docs: document codex-monitor update workflow"
```

### Task 8: Full Verification and Local Installation

**Files:**
- Inspect: all changed files
- Build output: `target/release/*.exe`

- [ ] **Step 1: Run formatting and whitespace checks**

Run: `cargo fmt --check`

Expected: PASS.

Run: `git diff --check main...HEAD`

Expected: no output and exit code 0.

- [ ] **Step 2: Run the complete test suite**

Run: `cargo test`

Expected: all unit and integration tests PASS.

- [ ] **Step 3: Run strict linting**

Run: `cargo clippy --all-targets -- -D warnings`

Expected: PASS with no warnings.

- [ ] **Step 4: Build all release executables**

Run: `cargo build --release --bins`

Expected: `target/release/codex-monitor.exe`, `target/release/cdxm.exe`, and
`target/release/cdxm-codex-app-bridge.exe` exist.

- [ ] **Step 5: Audit the implementation against the design**

Read `docs/superpowers/specs/2026-07-10-codex-monitor-update-command-design.md`
requirement by requirement. Confirm current code or test evidence for public UX,
three-binary release payload, checksum enforcement, App shutdown preflight,
runtime refresh, fixed destinations, helper handoff, rollback, owned environment,
result reporting, docs, and non-Windows behavior. Treat any missing evidence as
remaining implementation work.

- [ ] **Step 6: Install the new command without disturbing the live App bridge**

When Codex App is still open, copy only the newly built `codex-monitor.exe` and
`cdxm.exe` into `~/.codex-monitor/bin`; do not replace the running bridge or
managed runtime. Verify:

```powershell
codex-monitor update --help
codex-monitor update
```

Expected: help succeeds, and the update attempt refuses before mutation with a
clear fully-quit-Codex-App message.

- [ ] **Step 7: Prepare live closed-App acceptance**

Build a checksum-protected Windows ZIP containing all three release executables
and verify the updater against an isolated install root and a local HTTP fixture.
This proves the full download, helper, transaction, and result path without
closing the active conversation. The final operator acceptance after a published
release is: fully quit App, run `codex-monitor update`, reopen App, and confirm
`cdxm targets` shows `codex-app-bridge`.

- [ ] **Step 8: Commit any verification-only fixes**

If verification required source fixes, stage only those files and commit with a
message describing the corrected behavior. Leave generated build artifacts and
temporary HTTP fixtures untracked and remove them before completion.
