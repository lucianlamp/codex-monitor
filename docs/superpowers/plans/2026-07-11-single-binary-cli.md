# Single-Binary CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship only the native `codex-monitor` executable while keeping `cdxm` as a compatibility launcher and safely retiring the old compiled `cdxm` binary.

**Architecture:** Cargo, release archives, and the updater manage one native file. Platform installers create a text launcher for `cdxm`; Windows puts it in the earlier `%USERPROFILE%\.agents\bin` PATH entry so new commands bypass a still-running legacy `cdxm.exe`. Exact-path migration removes the old EXE only after its consumers exit and never manages their lifecycle.

**Tech Stack:** Rust 2021, Cargo, PowerShell, POSIX shell, GitHub Actions, Rust integration tests.

---

### Task 1: Make Cargo and releases single-binary

**Files:**
- Modify: `Cargo.toml:12-18`
- Delete: `src/bin/cdxm.rs`
- Modify: `.github/workflows/release.yml:36-55`
- Modify: `tests/cli_contract.rs:4-12`
- Modify: `tests/release_workflow_contract.rs:35-50`

- [ ] **Step 1: Write failing package and release contracts**

Replace the binary contract with assertions that only `codex-monitor` is declared:

```rust
#[test]
fn package_exposes_one_native_binary() {
    let manifest = fs::read_to_string(repo_root().join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"codex-monitor\""));
    assert!(!manifest.contains("name = \"cdxm\""));
}
```

Update release workflow assertions so Unix archives name only `codex-monitor` and Windows archives copy only `codex-monitor.exe`:

```rust
assert!(wf.contains("release/codex-monitor.exe"));
assert!(!wf.contains("release/cdxm.exe"));
assert!(!wf.contains("codex-monitor cdxm"));
```

- [ ] **Step 2: Run contracts and verify failure**

Run:

```powershell
cargo test --test cli_contract package_exposes_one_native_binary
cargo test --test release_workflow_contract
```

Expected: failures because Cargo and release packaging still include `cdxm`.

- [ ] **Step 3: Remove the duplicate native target**

Delete the `[[bin]] name = "cdxm"` stanza and `src/bin/cdxm.rs`. Change release packaging to:

```bash
cp "target/${{ matrix.target }}/release/codex-monitor" "$staging/"
tar -czf "$name.tar.gz" -C "$staging" codex-monitor
```

```powershell
Copy-Item "target/${{ matrix.target }}/release/codex-monitor.exe" $staging
```

- [ ] **Step 4: Run package and release contracts**

Run:

```powershell
cargo test --test cli_contract
cargo test --test release_workflow_contract
```

Expected: all tests pass and Cargo builds one CLI binary.

- [ ] **Step 5: Commit**

```powershell
git add Cargo.toml src/bin/cdxm.rs .github/workflows/release.yml tests/cli_contract.rs tests/release_workflow_contract.rs
git commit -m "refactor: ship one native CLI binary"
```

### Task 2: Reduce updater manifests to `codex-monitor`

**Files:**
- Modify: `src/update/model.rs:10-45,165-205`
- Modify: `src/update/archive.rs:180-315`
- Modify: `src/update/apply.rs:380-470`
- Modify: `src/update.rs:120-175`
- Modify: `src/update/windows.rs:100-115,270-325,780-845`

- [ ] **Step 1: Write failing one-file updater tests**

Change the managed-file expectation to:

```rust
assert_eq!(ManagedFile::ALL, [ManagedFile::CodexMonitor]);
assert_eq!(ManagedFile::RELEASE, ManagedFile::ALL);
```

Change valid ZIP fixtures to one entry:

```rust
fn valid_entries() -> Vec<(&'static str, &'static [u8])> {
    vec![("codex-monitor.exe", b"monitor")]
}
```

Add rejection coverage for an archive that still contains `cdxm.exe` as an unexpected member.

- [ ] **Step 2: Run updater tests and verify failure**

Run:

```powershell
cargo test update::model::tests
cargo test update::archive::tests
cargo test update::apply::tests
```

Expected: failures because `ManagedFile::Cdxm` is still required.

- [ ] **Step 3: Remove `ManagedFile::Cdxm` and public-binary deferral**

Keep the enum and file sets as:

```rust
pub enum ManagedFile {
    CodexMonitor,
}

pub const ALL: [Self; 1] = [Self::CodexMonitor];
pub const RELEASE: [Self; 1] = Self::ALL;
```

Remove `defer_active_public_binaries`, its staging rewrite, and the deferred-public result message. The updater helper already runs outside the installed `codex-monitor.exe`, so the single managed file can use the normal transactional replacement.

- [ ] **Step 4: Run updater tests**

Run:

```powershell
cargo test update::
```

Expected: all updater model, archive, apply, and Windows tests pass.

- [ ] **Step 5: Commit**

```powershell
git add src/update.rs src/update/model.rs src/update/archive.rs src/update/apply.rs src/update/windows.rs
git commit -m "refactor: update one native executable"
```

### Task 3: Install a Unix `cdxm` launcher

**Files:**
- Modify: `install.sh:20-45,165-250,330-350`
- Modify: `tests/installer_contract.rs`

- [ ] **Step 1: Write failing Unix installer contracts**

Assert that release extraction requests only `codex-monitor`, source builds only that Cargo target, and the installer writes a forwarding launcher:

```rust
assert!(installer.contains("tar -xzf \"$dl_dir/$archive\" -C \"$extract_dir\" codex-monitor"));
assert!(installer.contains("cargo install --path \"$SOURCE_DIR\" --bin codex-monitor"));
assert!(installer.contains("exec \"$SCRIPT_DIR/codex-monitor\" \"$@\""));
```

Add a functional test that invokes the generated launcher against a fake sibling `codex-monitor`, verifies argument forwarding, and verifies a nonzero exit code is preserved.

- [ ] **Step 2: Run installer contracts and verify failure**

Run:

```powershell
cargo test --test installer_contract
```

Expected: failures because the installer still extracts and compiles two binaries.

- [ ] **Step 3: Implement the Unix launcher**

After publishing `codex-monitor`, write `$BIN_DIR/cdxm` as:

```sh
#!/usr/bin/env sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$SCRIPT_DIR/codex-monitor" "$@"
```

Install it with mode `0755`. Update prebuilt and source flows to handle only the native `codex-monitor` member.

- [ ] **Step 4: Run Unix installer contracts**

Run:

```powershell
cargo test --test installer_contract
```

Expected: launcher and existing installer contracts pass.

- [ ] **Step 5: Commit**

```powershell
git add install.sh tests/installer_contract.rs
git commit -m "feat: add Unix cdxm compatibility launcher"
```

### Task 4: Install the Windows CMD launcher and migrate old `cdxm.exe`

**Files:**
- Modify: `install.ps1:15-35,75-215,330-410`
- Modify: `src/update/windows.rs`
- Modify: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Write failing Windows launcher and migration contracts**

Require a fixed compatibility path and launcher body:

```rust
assert!(installer.contains("$CdxmCompatTarget = Join-Path $AgentsBin 'cdxm.cmd'"));
assert!(installer.contains("function Write-CdxmCompatibilityLauncher"));
assert!(installer.contains("codex-monitor.exe\" %*"));
assert!(!installer.contains("Join-Path $releaseDir 'cdxm.exe'"));
```

Add Rust Windows migration tests with an inventory containing an exact active legacy `bin\\cdxm.exe`: finalization must preserve the file and report deferral. With an empty inventory, finalization cleanup must remove that fixed old file. Neither test may contain or invoke a stop command.

- [ ] **Step 2: Run Windows contracts and verify failure**

Run:

```powershell
cargo test --test windows_installer_contract
cargo test update::windows::tests
```

Expected: failures because Windows still publishes and updater-manages `cdxm.exe`.

- [ ] **Step 3: Implement CMD publication**

Write `%USERPROFILE%\.agents\bin\cdxm.cmd` atomically with this effective body:

```bat
@echo off
"%USERPROFILE%\.codex-monitor\bin\codex-monitor.exe" %*
exit /b %ERRORLEVEL%
```

Use the existing agents-bin-first PATH normalization. Prebuilt extraction and source builds publish only `codex-monitor.exe`.

- [ ] **Step 4: Implement exact-path old EXE cleanup**

Treat `%USERPROFILE%\.codex-monitor\bin\cdxm.exe` as a fixed obsolete file, separate from the old App bridge runtime set. Query `Win32_Process.ExecutablePath` exactly. If active, warn and keep it; otherwise remove it. Refresh `cdxm.cmd` during both installer and updater finalization. Do not stop any PID.

- [ ] **Step 5: Run Windows contracts**

Run:

```powershell
$errors=$null; $tokens=$null
[System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path .\install.ps1), [ref]$tokens, [ref]$errors) | Out-Null
if ($errors.Count) { $errors; exit 1 }
cargo test --test windows_installer_contract
cargo test update::windows::tests
```

Expected: parser succeeds and all migration/installer tests pass.

- [ ] **Step 6: Commit**

```powershell
git add install.ps1 src/update/windows.rs tests/windows_installer_contract.rs
git commit -m "feat: add Windows cdxm compatibility launcher"
```

### Task 5: Update documentation and verify the complete migration

**Files:**
- Modify: `README.md`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `skills/codex-monitor/references/codex-monitor-operations.md`
- Modify: `docs/superpowers/specs/2026-07-11-single-binary-cli-design.md` only if implementation reveals a contradiction

- [ ] **Step 1: Update user-facing installation language**

Document one native executable, the `cdxm` compatibility launcher, Cargo-only behavior, and deferred exact-path cleanup. Keep command examples using `cdxm` because the alias remains supported.

- [ ] **Step 2: Run complete static and regression verification**

Run:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: formatting succeeds, all unit/integration/doc tests pass, and Clippy reports no warnings.

- [ ] **Step 3: Install from the worktree without stopping existing processes**

Run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Yes -Source . -NoShim -BuildFromSource
```

Expected: `codex-monitor.exe`, installed skill, and `.agents\\bin\\cdxm.cmd` update successfully. Exact active legacy `cdxm.exe` PIDs remain alive and its file is reported as deferred.

- [ ] **Step 4: Verify live command precedence and behavior**

Run:

```powershell
Get-Command cdxm -All
cdxm --help
codex-monitor --help
```

Expected: `cdxm.cmd` appears before the legacy `cdxm.exe`; both help commands reach the same installed native executable. Re-read the previously recorded watcher PIDs and verify none were stopped.

- [ ] **Step 5: Verify native App behavior**

Run the installed foreground helper with a self-addressed agmsg test event, confirm the current task receives it, and verify native in-app Browser navigation to Google returns title `Google` and a non-empty DOM snapshot. Do not start or replace a watcher.

- [ ] **Step 6: Commit documentation and final fixes**

```powershell
git add README.md skills/codex-monitor docs/superpowers/specs/2026-07-11-single-binary-cli-design.md
git commit -m "docs: explain single-binary CLI installation"
```
