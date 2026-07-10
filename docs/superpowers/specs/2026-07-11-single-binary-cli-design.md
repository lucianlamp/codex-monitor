# Single-Binary CLI Design

## Goal

Distribute, install, and update only one native executable,
`codex-monitor` (`codex-monitor.exe` on Windows), while preserving the familiar
`cdxm` command through a small launcher that forwards every argument and exit
code to the native executable.

## Chosen approach

Use one native executable plus a text launcher.

- `codex-monitor` remains the only Rust binary target and native release asset.
- `cdxm` remains a command name, not a second compiled executable.
- Windows installs `cdxm.cmd` in `%USERPROFILE%\.agents\bin`, which is ordered
  before `%USERPROFILE%\.codex-monitor\bin` on the managed user `PATH`.
- macOS and Linux install an executable shell launcher at
  `$HOME/.codex-monitor/bin/cdxm` beside the native `codex-monitor` binary.
- Both launchers forward arguments unchanged and return the native process exit
  code.

This keeps existing scripts and documentation usable while eliminating duplicate
native payloads and independent binary locks.

## Rejected alternatives

### Keep both compiled binaries

This preserves compatibility but retains duplicate release payloads, duplicate
update state, and the Windows locking problem that motivated this change.

### Remove `cdxm` without an alias

This is the smallest installation but breaks existing watcher definitions,
scripts, documentation, and user muscle memory. The compatibility launcher costs
only a few lines and avoids that break.

## Packaging and installation

`Cargo.toml` exposes only the `codex-monitor` binary. Release archives contain
exactly one native member named `codex-monitor` or `codex-monitor.exe`.

The Unix installer installs that member, then writes a POSIX launcher which
resolves its own directory and executes the sibling `codex-monitor`. Source
installation uses `cargo install --bin codex-monitor` and writes the same
launcher.

The Windows installer publishes only `codex-monitor.exe`, then atomically writes
`%USERPROFILE%\.agents\bin\cdxm.cmd` with an absolute invocation of
`%USERPROFILE%\.codex-monitor\bin\codex-monitor.exe`. It keeps the existing PATH
order so new `cdxm` commands resolve to the CMD launcher even while a legacy
`%USERPROFILE%\.codex-monitor\bin\cdxm.exe` process is still running.

## Legacy Windows migration

The old `cdxm.exe` is an obsolete fixed path.

- The installer and updater enumerate exact executable paths only.
- They never stop, kill, restart, or replace a process.
- If old `cdxm.exe` is active, its file remains until a later installer or
  updater run.
- If it is inactive, the fixed old file is removed.
- The compatibility CMD is installed regardless, so newly launched `cdxm`
  commands use the single native `codex-monitor.exe` immediately.

This migration does not change `CODEX_CLI_PATH`, watcher ownership, or any
existing process command line.

## Updater model

The manifest has one required managed file: `CodexMonitor`. The archive verifier,
transactional apply layer, and installed-state verification all expect exactly
that one file. The Windows finalization step refreshes `cdxm.cmd` and removes an
inactive legacy `cdxm.exe`; an active legacy file is reported as deferred but is
not treated as an update failure.

Because the updater helper runs from staging after the parent native executable
exits, it can replace `codex-monitor.exe` normally. There is no second public
binary whose watcher lock can block the transaction.

## Command behavior

`codex-monitor ...` and `cdxm ...` reach the same `run_cli_blocking()` entrypoint.
Help text may identify the program as `codex-monitor`; command parsing, output,
exit status, and environment behavior are otherwise identical.

## Documentation

User-facing examples may continue using the shorter `cdxm` command. Installation
and release documentation must state that `cdxm` is a compatibility launcher and
that only `codex-monitor` is a native binary. Cargo-only installation documents
`codex-monitor` as the available command; users who want the compatibility alias
use `install.sh` or `install.ps1`.

## Verification

Automated contracts cover:

- one Cargo binary target;
- one native member in every release archive;
- one updater-managed file;
- Unix launcher argument and exit-code forwarding;
- Windows CMD content and PATH precedence;
- active legacy `cdxm.exe` deferral without process management;
- inactive legacy `cdxm.exe` removal;
- full CLI, installer, updater, release, formatting, and Clippy regression tests.

Live Windows acceptance checks that `Get-Command cdxm -All` resolves the CMD
launcher before the still-running old EXE, `cdxm --help` executes the installed
`codex-monitor.exe`, and no existing watcher PID is stopped.
