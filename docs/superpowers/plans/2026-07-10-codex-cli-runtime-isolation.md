# Codex CLI Runtime Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the npm Codex CLI the only automatic terminal runtime while keeping the Codex App runtime private and removing the stale Desktop CLI directory from the user PATH.

**Architecture:** The public Bash shim fails closed when only the Windows Desktop CLI is available. The Windows installer and updater independently normalize the user PATH with the same ordering contract and save the original PATH once for rollback. The App bridge continues to resolve only its staged private runtime.

**Tech Stack:** Bash, PowerShell 5.1, Rust, Cargo integration tests, Windows environment variables

---

## File map

- `skills/codex-monitor/scripts/codex-shim.sh`: public CLI candidate selection and error reporting.
- `tests/shim_resolution.rs`: executable resolution regression tests.
- `install.ps1`: initial PATH backup and idempotent user/process PATH normalization.
- `tests/windows_installer_contract.rs`: installer source contract coverage.
- `src/update/windows.rs`: updater inventory, pure PATH normalization, backup persistence, and user environment update.
- `docs/superpowers/specs/2026-07-10-codex-cli-runtime-isolation-design.md`: rollback contract clarified by implementation discovery.

### Task 1: Make public shim resolution fail closed

**Files:**
- Modify: `tests/shim_resolution.rs`
- Modify: `skills/codex-monitor/scripts/codex-shim.sh`

- [ ] **Step 1: Replace the Desktop-only success test with a failing-resolution assertion**

Add a `run_shim_output` helper returning `std::process::Output`, keep
`run_shim` as the success wrapper, and replace the Desktop-only test with:

```rust
#[test]
fn shim_rejects_desktop_bundle_as_the_only_real_codex() {
    let temp = TempDir::new().unwrap();
    let local_app_data = temp.path().join("LocalAppData");
    let desktop_dir = local_app_data.join("OpenAI/Codex/bin");
    write_fake_codex(&desktop_dir.join("codex"), "desktop-old");

    let output = run_shim_output(&[&desktop_dir], &local_app_data);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("refusing Windows Desktop Codex fallback"));
}
```

- [ ] **Step 2: Run the focused test and verify the old fallback makes it fail**

Run: `cargo test --test shim_resolution shim_rejects_desktop_bundle_as_the_only_real_codex -- --exact`

Expected: FAIL because the existing shim exits successfully with
`desktop-old`.

- [ ] **Step 3: Remove the Desktop fallback from automatic selection**

Track whether a Desktop candidate was skipped, never return it, and emit an
actionable error:

```bash
local self_dir self_path shim_target old_ifs path_dir candidate candidate_dir candidate_path desktop_seen
desktop_seen=0

# Inside the candidate branch:
if is_desktop_codex_candidate "$candidate_path"; then
  desktop_seen=1
  continue
fi
printf '%s\n' "$candidate_path"
return 0

# After the PATH loop:
if [ "$desktop_seen" -eq 1 ]; then
  echo "codex-monitor shim: refusing Windows Desktop Codex fallback; install @openai/codex with npm or set CODEX_MONITOR_REAL_CODEX" >&2
else
  echo "codex-monitor shim: real codex not found on PATH; install @openai/codex with npm or set CODEX_MONITOR_REAL_CODEX" >&2
fi
return 1
```

- [ ] **Step 4: Run all shim resolution tests**

Run: `cargo test --test shim_resolution`

Expected: all shim tests PASS; Desktop-before-npm still selects npm and
Desktop-only exits non-zero.

- [ ] **Step 5: Commit the shim change**

```powershell
git add -- skills/codex-monitor/scripts/codex-shim.sh tests/shim_resolution.rs
git commit -m "fix: reject stale Desktop CLI fallback"
```

### Task 2: Normalize PATH in the Windows installer

**Files:**
- Modify: `install.ps1`
- Modify: `tests/windows_installer_contract.rs`

- [ ] **Step 1: Add failing installer contract assertions**

Require the installer to contain the backup and normalization boundaries:

```rust
#[test]
fn windows_installer_isolates_the_public_cli_path() {
    let installer = installer_source();
    assert!(installer.contains("user-path-backup.json"));
    assert!(installer.contains("function Get-CdxmNormalizedPath"));
    assert!(installer.contains("function Repair-CdxmUserPath"));
    assert!(installer.contains("OpenAI\\Codex\\bin"));
    assert!(installer.contains("Join-Path $env:APPDATA 'npm'"));
    assert!(installer.contains("Repair-CdxmUserPath"));
}
```

- [ ] **Step 2: Run the focused installer contract test**

Run: `cargo test --test windows_installer_contract windows_installer_isolates_the_public_cli_path -- --exact`

Expected: FAIL because the installer only prepends individual PATH entries.

- [ ] **Step 3: Add pure normalization and one-time backup functions**

Replace `Add-UserPathEntry` with `Get-CdxmNormalizedPath` and
`Repair-CdxmUserPath`. The normalization function must:

```powershell
function Get-CdxmNormalizedPath {
    param(
        [AllowNull()][string]$Current,
        [string[]]$Preferred,
        [string[]]$Removed
    )
    $result = [Collections.Generic.List[string]]::new()
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in @($Preferred) + @($Current -split ';')) {
        if ([string]::IsNullOrWhiteSpace($entry)) { continue }
        $trimmed = $entry.Trim().TrimEnd('\', '/')
        if ($Removed | Where-Object { $_.TrimEnd('\', '/') -ieq $trimmed }) { continue }
        if ($seen.Add($trimmed)) { $result.Add($entry.Trim()) }
    }
    return ($result -join ';')
}
```

`Repair-CdxmUserPath` must save the original user PATH once to
`$InstallRoot\user-path-backup.json`, calculate preferred entries as
`$AgentsBin`, `$BinDir`, and `%APPDATA%\npm`, remove
`%LOCALAPPDATA%\OpenAI\Codex\bin`, then update both the User PATH and current
process PATH.

- [ ] **Step 4: Replace the two prepend calls with one repair call**

Under the existing `-NoPath` and confirmation boundary, invoke:

```powershell
Repair-CdxmUserPath
```

Do not mutate PATH when `-NoPath` is supplied.

- [ ] **Step 5: Run installer contract tests**

Run: `cargo test --test windows_installer_contract`

Expected: all installer contract tests PASS.

- [ ] **Step 6: Commit the installer change**

```powershell
git add -- install.ps1 tests/windows_installer_contract.rs
git commit -m "fix: normalize Windows Codex CLI path"
```

### Task 3: Reassert PATH during `codex-monitor update`

**Files:**
- Modify: `src/update/windows.rs`

- [ ] **Step 1: Add pure PATH normalization unit tests**

Add tests proving managed entries are ordered, Desktop is removed, unrelated
entries keep their order, and repeated normalization is idempotent:

```rust
#[test]
fn public_cli_path_is_ordered_and_desktop_is_removed() {
    let actual = normalize_user_path(
        Some(r"C:\Tools;C:\Users\me\AppData\Local\OpenAI\Codex\bin;C:\Users\me\AppData\Roaming\npm"),
        &[
            PathBuf::from(r"C:\Users\me\.agents\bin"),
            PathBuf::from(r"C:\Users\me\.codex-monitor\bin"),
            PathBuf::from(r"C:\Users\me\AppData\Roaming\npm"),
        ],
        &[PathBuf::from(r"C:\Users\me\AppData\Local\OpenAI\Codex\bin")],
    );
    assert_eq!(actual, r"C:\Users\me\.agents\bin;C:\Users\me\.codex-monitor\bin;C:\Users\me\AppData\Roaming\npm;C:\Tools");
    assert_eq!(normalize_user_path(Some(&actual), &preferred(), &removed()), actual);
}
```

- [ ] **Step 2: Run the focused updater test and verify it fails to compile**

Run: `cargo test update::windows::tests::public_cli_path_is_ordered_and_desktop_is_removed`

Expected: FAIL because `normalize_user_path` does not exist.

- [ ] **Step 3: Extend inventory and add pure normalization**

Add `userPath` to `INVENTORY_SCRIPT` and `user_path: Option<String>` to
`WindowsInventory`. Implement `normalize_user_path` by splitting on `;`,
comparing slash-normalized and trailing-separator-trimmed entries
case-insensitively, prepending each preferred entry once, removing every
Desktop entry, and preserving all unrelated entries in original order.

- [ ] **Step 4: Persist the original PATH once and update the owned environment**

In `reassert_owned_environment`:

1. Derive `~/.agents/bin`, `<install-root>/bin`, `%APPDATA%/npm`, and
   `%LOCALAPPDATA%/OpenAI/Codex/bin`.
2. Write `{ "version": 1, "userPath": <original> }` atomically to
   `<install-root>/user-path-backup.json` only when it does not exist.
3. Pass the normalized PATH through `CDXM_UPDATE_USER_PATH`.
4. Extend the PowerShell environment update with:

```powershell
[Environment]::SetEnvironmentVariable('Path', $env:CDXM_UPDATE_USER_PATH, 'User')
```

- [ ] **Step 5: Run updater tests**

Run: `cargo test update::windows`

Expected: all Windows updater unit tests PASS.

- [ ] **Step 6: Commit the updater change**

```powershell
git add -- src/update/windows.rs
git commit -m "fix: reassert public Codex CLI path on update"
```

### Task 4: Verify and install without disturbing live runtimes

**Files:**
- Modify: `docs/superpowers/specs/2026-07-10-codex-cli-runtime-isolation-design.md`

- [ ] **Step 1: Commit the design clarification and implementation plan**

```powershell
git add -- docs/superpowers/specs/2026-07-10-codex-cli-runtime-isolation-design.md docs/superpowers/plans/2026-07-10-codex-cli-runtime-isolation.md
git commit -m "docs: plan Codex CLI runtime isolation"
```

- [ ] **Step 2: Run the complete repository verification surface**

Run:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: all commands exit `0` with no failing tests or warnings.

- [ ] **Step 3: Install the updated skill and shim without replacing the live App bridge**

Run the repository installer with the source checkout and shim enabled, but
without App bridge replacement:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Yes -InstallShim -Source .
```

Expected: installed skill and shim are refreshed, user PATH is normalized and
backed up, and the running App bridge is not stopped or replaced.

- [ ] **Step 4: Verify command selection and live process isolation**

Run:

```powershell
Get-Command codex -All
& "$env:APPDATA\npm\codex.cmd" --version
Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -in @(35600,32080,57184,80724) }
```

Expected: the shim is first, npm remains the real terminal CLI, the Desktop CLI
directory is absent from the user PATH, and the existing CLI/App processes are
unchanged.

- [ ] **Step 5: Launch a fresh isolated CLI smoke and verify npm ownership**

Launch through `~/.agents/bin/codex.cmd` with a temporary shim run directory,
then verify the TUI command line points under `%APPDATA%\npm\node_modules` and
its loopback app-server connection is established. Stop only that temporary
smoke process after verification.

- [ ] **Step 6: Record final status without claiming Browser policy repair**

Report the public CLI/runtime isolation result separately from the observed
Browser enterprise-policy rejection. Do not treat Google access as a passing
acceptance criterion for this local change.
