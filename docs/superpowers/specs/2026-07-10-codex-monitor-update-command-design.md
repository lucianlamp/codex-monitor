# `codex-monitor update` Design

## Goal

Provide one Windows command that can be run from any working directory after
Codex App is fully closed:

```powershell
codex-monitor update
```

The command updates both the installed codex-monitor binaries and the private
Codex App runtime used by the shared App bridge. It replaces the current
checkout-dependent `install.ps1 -SkipBuild -InstallAppBridge` maintenance step.

## Scope

The first implementation is Windows-only because refreshing the private Codex
App runtime and replacing in-use `.exe` files are Windows-specific operations.
It updates these installed binaries:

- `codex-monitor.exe`
- `cdxm.exe`
- `cdxm-codex-app-bridge.exe`

It also refreshes these files from the currently installed Codex App package:

- `codex-app-real.exe`
- `codex-code-mode-host.exe`
- `codex-command-runner.exe`, when present
- `codex-windows-sandbox-setup.exe`, when present

The command preserves and reasserts codex-monitor's existing user environment
configuration for `CODEX_CLI_PATH` and `CDXM_REAL_CODEX`. It does not start or
stop watchers, launch Codex App, modify a source checkout, or replace the
installed Codex skill.

## Considered Approaches

### Selected: native staged updater

Add a first-class `update` subcommand. The running CLI discovers the installed
Codex App, downloads and validates the latest release, stages every destination
file, then hands the staged update to a temporary updater process. The helper
waits for the invoking executable to exit before replacing files.

This gives the requested one-command UX without requiring a repository checkout
and provides one transaction boundary for the CLI, bridge, and App runtime.

### Rejected: download and invoke `install.ps1`

This would reuse existing code, but it would make the installed command depend
on a downloaded PowerShell script and retain the current self-replacement and
checkout-discovery problems. It would also split validation and rollback across
two implementations.

### Rejected: permanent launcher/updater executable

A stable launcher could replace versioned payloads while remaining unchanged,
but it adds another installed binary and requires a broader installation-layout
migration. A temporary helper provides the needed Windows handoff with less
persistent complexity.

## Command Flow

### 1. Preflight

Before changing any destination, the command:

1. Resolves the install root, defaulting to `~/.codex-monitor` and honoring
   `CDXM_INSTALL_ROOT`.
2. Confirms that the current installation owns the configured App bridge.
3. Refuses to continue while a live Codex App bridge marker or Codex App process
   indicates that the App is still running.
4. Resolves the currently installed Codex App executable and its matching
   companion binaries.
5. Ensures the install and runtime destinations are writable.

Failure at this stage is read-only and prints the corrective action: fully quit
Codex App and rerun the command from PowerShell.

### 2. Download and validation

The command downloads the latest Windows release archive and its `.sha256`
sidecar from the existing release base, honoring
`CDXM_INSTALL_RELEASE_BASE`. The checksum is mandatory and a mismatch or
missing sidecar is fatal.

Archive extraction uses an exact top-level allowlist. The archive must contain
exactly one copy of each of the three installed executables. Nested paths,
duplicate names, missing files, and unexpected executable entries are rejected.

The resolved Codex App runtime files are copied into the same staging directory.
Every staged file is hashed before the apply phase. Optional companions remain
optional, but the real Codex executable and code-mode host are required.

### 3. Handoff and apply

The invoking process copies its updater-capable executable into the staging
directory and launches a hidden internal apply command with:

- the invoking process ID;
- the staging directory;
- the install root;
- the expected staged hashes;
- a result-file path.

The invoking process exits only after the helper was created successfully. The
helper inherits the console, waits for the invoking process to terminate,
rechecks that Codex App is closed, and then applies the staged files.

The helper creates same-directory backups before replacement so each rename
stays on the destination volume. It replaces the bridge and runtime as one
logical set, then `cdxm.exe`, and finally `codex-monitor.exe`. Identical files
are skipped. The helper writes a durable success or failure result and prints a
concise final status before cleaning its staging directory.

On the next `codex-monitor` invocation, any unreported failed result is surfaced
before normal command handling. Successful result metadata may be removed once
reported.

### 4. Environment configuration

The existing `app-bridge-env.json` remains the source of rollback ownership.
An update never overwrites the original pre-bridge environment snapshot. After
files are replaced, the updater reasserts these user-level values:

- `CODEX_CLI_PATH=<install-root>\bin\cdxm-codex-app-bridge.exe`
- `CDXM_REAL_CODEX=<install-root>\runtime\codex-app-real.exe`

The success message tells the user to reopen Codex App. Watcher lifecycle is
outside this command's scope.

## Transaction and Rollback

No installed file is changed until all release and App-runtime inputs have been
staged and validated. During apply:

1. Existing destinations are renamed to unique backup names.
2. Staged files are renamed or copied into their final locations.
3. Installed hashes are compared with the staged manifest.
4. Backups are deleted only after the complete set verifies.

If any replacement or verification fails, the helper restores all destinations
already touched in that transaction. Failure details remain in the result file,
and staging is retained only when needed for diagnosis or rollback recovery.

The updater never edits the packaged Codex App directory.

## Release and Installer Changes

The Windows release archive currently contains only `codex-monitor.exe` and
`cdxm.exe`. Release packaging must add `cdxm-codex-app-bridge.exe` and continue
publishing a checksum for the complete archive.

`install.ps1` must accept and validate the same three-file archive allowlist so
fresh prebuilt installations and `codex-monitor update` produce the same binary
set. Source-build installation behavior remains unchanged.

## Error Handling

- Running Codex App: fail before mutation with an explicit close-and-retry
  message.
- No owned bridge configuration: fail with the one-time bridge installation
  command instead of taking over unrelated environment settings.
- Missing required Codex runtime companion: fail before mutation and name the
  missing file.
- Network, checksum, or archive validation failure: discard staging and leave
  the installation unchanged.
- Helper launch failure: leave the installation unchanged and clean staging.
- Apply failure: restore backups, persist the error result, and return a
  corrective diagnostic on the next invocation.
- Non-Windows platform: report that combined App-runtime update is currently
  Windows-only.

## Security

- Release checksums are mandatory; there is no unverified source-build fallback
  inside `update`.
- Only exact expected archive members are extracted.
- All paths in the apply manifest are derived from the resolved install root and
  fixed filenames; the manifest cannot name arbitrary destinations.
- App runtime inputs must resolve inside the installed Codex App package.
- Environment values are changed only when the existing configuration is owned
  by codex-monitor.

## Verification

Automated tests must cover:

- CLI parsing and help for `codex-monitor update`;
- exact archive allowlisting, required members, duplicates, and checksum
  failures;
- App-running and bridge-ownership preflight failures without mutation;
- runtime staging with required and optional companions;
- idempotent identical-file updates;
- successful apply ordering and hash verification;
- rollback after a replacement failure;
- persisted helper-result reporting;
- non-Windows rejection;
- release and installer inclusion of the App bridge executable.

Repository verification is:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

Live acceptance is performed from an external PowerShell after fully quitting
Codex App. It must prove that one `codex-monitor update` invocation refreshes
the three installed binaries and the private App runtime, preserves the owned
environment values, and allows the reopened App to publish a live
`codex-app-bridge` target.
