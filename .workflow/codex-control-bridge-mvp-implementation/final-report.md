# Final Report: codex-control-bridge MVP implementation

## Outcome

MVP implementation is complete in the local branch. The crate now includes the
app-server protocol client, transports, target resolution, agmsg source adapter,
foreground delivery loop, CLI commands, fake app-server tests, README, and CI.

## Accepted Results

- Fixed naming contract: package `codex-control-bridge`, library
  `codex_control_bridge`, binaries `codex-control-bridge` and `ccb`.
- `clientInfo.name` is `codex-control-bridge`; `clientInfo.title` is
  `Codex Control Bridge`.
- Implemented app-server `initialize`, `initialized`, `thread/list`,
  `thread/read`, `turn/start`, and `thread/inject_items` builders.
- Implemented `AppServerClient` over `AppServerTransport`.
- Implemented memory, WebSocket, stdio, and Unix transports; Unix code is
  behind `cfg(unix)`.
- Implemented target selection for managed, app, explicit `ws://`, and
  `stdio://`.
- Implemented `ccb threads`, `ccb send`, and `ccb agmsg watch`.
- Implemented direct SQLite agmsg adapter without PATH shims, SessionStart
  hooks, `inbox.sh`, or `watch.sh`.
- Added JSON state persistence and last-seen tracking.
- Added fake app-server integration tests for threads and send.

## Rejected Results

- No live mutating Codex thread smoke was run.
- No watcher or agmsg lifecycle script was started.
- No PATH shim, SessionStart hook, or Codex App daemon replacement was added.

## Conflicts Resolved

- Tokio integration tests initially hung because synchronous subprocess waits
  blocked a current-thread runtime; fake app-server tests now use a multi-thread
  runtime.
- `directories::ProjectDirs::state_dir()` returns `Option<&Path>` in
  directories 6.0.0; state path resolution now falls back to `cache_dir()`.

## Verification Evidence

- `cargo fmt --check`: passed.
- `cargo test`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo check --target x86_64-pc-windows-msvc`: environment failure on this
  macOS host because the MSVC C standard headers/toolchain are unavailable for
  `libsqlite3-sys`; the Rust target is installed and CI runs the check on
  Windows.
- Workflow verifier passed for `.workflow/codex-control-bridge-mvp-implementation`.

## Remaining Risks

- Windows build should be confirmed on the configured Windows CI runner.
- Live `--target app threads` smoke is read-only and optional; live `send`
  requires explicit user approval.

## Reusable Follow-up

- Add a non-mutating live App attach smoke command when the user asks.
- Consider a lock file for concurrent state writers if a daemon mode is added.
