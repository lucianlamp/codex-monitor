# Final Report: codex-monitor MVP implementation

## Outcome

MVP implementation is complete in the local branch. The crate now includes the
app-server protocol client, transports, target resolution, agmsg source adapter,
foreground delivery loop, CLI commands, fake app-server tests, README, and CI.

## Accepted Results

- Fixed naming contract: package `codex-monitor`, library
  `codex_monitor`, binaries `codex-monitor` and `cdxm`.
- `clientInfo.name` is `codex-monitor`; `clientInfo.title` is
  `Codex Monitor`.
- Implemented app-server `initialize`, `initialized`, `thread/list`,
  `thread/read`, `turn/start`, and `thread/inject_items` builders.
- Implemented `AppServerClient` over `AppServerTransport`.
- Implemented memory, WebSocket, stdio, and Unix transports; Unix code is
  behind `cfg(unix)`.
- Implemented target selection for managed, app, explicit `ws://`, and
  `stdio://`.
- Implemented `cdxm threads`, `cdxm send`, and `cdxm agmsg watch`.
- Implemented direct SQLite agmsg adapter without PATH shims, SessionStart
  hooks, `inbox.sh`, or `watch.sh`.
- Windows target excludes the SQLite C dependency and returns a clear
  unsupported error for `agmsg watch`; ws/stdio CLI targets compile on Windows.
- Added JSON state persistence and last-seen tracking.
- Added fake app-server integration tests for threads and send.
- Added real app-server `thread/list` response parsing for the current
  `data`/`name`/`preview` schema.
- Verified real managed loopback `cdxm threads` smoke.
- Added explicit transport close and managed child cleanup on foreground
  commands and watch shutdown.

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
- Current app-server `thread/list` returns `data` rather than the older
  `threads`/`items` shape; parser and fake server tests now cover the current
  schema.

## Verification Evidence

- `cargo fmt --check`: passed.
- `cargo test`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo check --target x86_64-pc-windows-msvc`: passed.
- `cargo check --tests --target x86_64-pc-windows-msvc`: passed.
- `cargo run --bin cdxm -- threads --cwd /Users/ysk411/dev/codex-monitor`:
  passed with exit 0 against a real managed loopback app-server.
- Managed child cleanup check printed no `codex app-server --listen
  ws://127.0.0.1` process after the smoke command.
- Workflow verifier passed for `.workflow/codex-monitor-mvp-implementation`.

## Remaining Risks

- Live `--target app threads` App daemon attach smoke is read-only and optional; live `send`
  requires explicit user approval.

## Reusable Follow-up

- Add a non-mutating live App attach smoke command when the user asks.
- Consider a lock file for concurrent state writers if a daemon mode is added.
