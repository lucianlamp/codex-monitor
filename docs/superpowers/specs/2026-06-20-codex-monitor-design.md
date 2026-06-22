# Codex Monitor Design Spec

Status: active design for the current Rust implementation. The repository now
contains a working MVP; this document defines the stable core boundary to keep
future adapter work from drifting back into source-specific monitor logic.

Date: 2026-06-20

## Goal

Build `codex-monitor`, a local-first bridge that delivers external
events into the Codex App / Codex app-server control plane without depending on
Codex startup shims, shell PATH replacement, or SessionStart hooks.

The first source adapter is agmsg, but the core product is not an agmsg tool.
The core owns Codex app-server protocol, transport, target resolution, delivery
policy, event normalization, and CLI plumbing. Source adapters own only source
specific event discovery and formatting.

## Required names

- Repository and directory: `codex-monitor`
- Cargo package name: `codex-monitor`
- Primary binary: `codex-monitor`
- Alias binary: `cdxm`
- Library crate import name: `codex_monitor`
- Codex app-server `clientInfo.name`: `codex-monitor`
- Codex app-server `clientInfo.title`: `Codex Monitor`

## Evidence checked before design

- Current directory `/Users/ysk411/dev/codex-monitor` was empty and was
  initialized as a git repository for the design document.
- `codex --version` returned `codex-cli 0.141.0`.
- `codex app-server --help` lists `--listen` values: `stdio://`,
  `unix://`, `unix://PATH`, `ws://IP:PORT`, and `off`.
- `codex app-server generate-json-schema --experimental --out $TMPDIR` confirmed
  current schema entries for `initialize`, `initialized`, `thread/list`,
  `thread/read`, `thread/inject_items`, `turn/start`, `turn/started`, and
  `turn/completed`.
- Generated schema params confirmed:
  - `thread/list` supports a `cwd` filter and pagination/sort fields.
  - `thread/read` requires `threadId` and supports `includeTurns`.
  - `turn/start` requires `threadId` and `input`.
  - `thread/inject_items` requires `threadId` and raw `items`.
  - `initialize` requires `clientInfo` and accepts optional capabilities.
- Existing local references show working JSON-RPC style app-server clients:
  - `/Users/ysk411/dev/agmsg-codex-app-server/codex-app-server-agent.mjs`
  - `/Users/ysk411/dev/codexa/src/protocol.rs`
  - `/Users/ysk411/dev/agmsg/scripts/codex-monitor.sh`

## POC facts carried into this spec

- The Codex App managed daemon socket at
  `$HOME/.codex/app-server-control/app-server-control.sock` can be reached by a
  WebSocket-over-Unix connection.
- `initialize`, `thread/list`, and `thread/read` worked against that daemon.
- Thread `019ee30d-3826-7ad0-bd26-ea91f5ca7733` was readable in the POC.
- The protocol surface includes `turn/start` and `thread/inject_items`.
- Cross-platform operation must prefer loopback WebSocket. Existing macOS App
  attach must remain a Unix socket fallback.
- Existing App UI attach and bridge-managed app-server sessions are separate
  target realms and must not be conflated.

## Non-goals

- Do not build a TUI.
- Do not replace Codex itself or patch Codex app-server internals.
- Do not make the core depend on agmsg naming, agmsg environment variables, or
  agmsg hook lifecycle.
- Do not auto-approve Codex server requests such as command or file-change
  approvals.
- Do not expose non-loopback WebSocket listeners in the MVP.
- Do not make `thread/inject_items` the default delivery path; it remains an
  explicit optional capability because it appends raw model-visible history.

## Approaches considered

### A. agmsg-specific Codex shim replacement

This would directly replace the current agmsg Codex bridge and optimize only
for `watch.sh`, `SessionStart`, and role-specific Codex sessions.

It is rejected because it preserves the wrong product boundary. The result
would still be an agmsg tool and would not become a reusable bridge for other
event sources.

### B. Generic bridge core with source adapters

This is the selected approach. The core handles Codex app-server protocol,
transports, targets, delivery semantics, queueing, safety, and CLI behavior.
agmsg is implemented as the first source adapter behind a small source event
interface.

This directly supports the objective: replace shim/PATH/SessionStart coupling
while keeping the bridge usable for later adapters.

### C. Long-running daemon first

This would build a persistent local service first, then add CLI commands as a
thin control client.

It is deferred. It may become useful after the MVP, but the first version
should stay testable as a normal CLI process with explicit foreground commands.

## Selected architecture

```text
thread detector
  -> source adapter(s)
  -> BridgeEvent records
  -> monitor delivery core and state cursor
  -> Codex app-server client
  -> transport implementation
  -> loaded Codex App / Codex CLI thread
```

The crate is a Rust CLI/library project. The package name stays hyphenated
(`codex-monitor`), while the Rust library crate is imported as
`codex_monitor`.

The implementation should keep protocol construction and transport mechanics in
the library so both binaries call the same code path. `cdxm` is an alias binary,
not a second implementation.

## Module boundaries

The implementation plan should use these boundaries unless a later protocol
check proves a better split:

- `cli`: clap command parsing, common endpoint options, exit codes.
- `protocol`: request builders, response classifiers, server notification
  types, and schema-shaped payload helpers.
- `client`: request id allocation, pending request tracking, initialize
  handshake, turn lifecycle, and server request handling.
- `transport`: async transport trait plus `ws`, `unix`, and `stdio`
  implementations.
- `target`: endpoint mode selection and thread resolution.
- `delivery`: monitor loop, cursor persistence, delivery mode, app-server ack
  handling, inject-items guardrails, and retry rules. It must not contain
  source-specific formatting or DB parsing.
- `sources`: source adapter trait and source event model. A source adapter owns
  event discovery and source-specific turn formatting.
- `sources::agmsg`: first adapter for agmsg message events.
- `state`: local state directory resolution and persisted seen-event registry.
- `bin`: primary binary and alias binary entrypoints.

## Core adapter contract

Codex Monitor has three core extension points:

1. Thread detection: `target` discovers live app-server endpoints and resolves
   a loaded thread by cwd or explicit thread id. It refuses to call
   `thread/resume` implicitly.
2. Message/event monitoring: a source adapter implements `BridgeEventSource` and
   returns monotonically ordered `BridgeEvent` records from a source cursor. The
   first adapter is `sources::agmsg`, backed by the agmsg SQLite message DB.
3. Notification delivery: `delivery::run_monitor_watch` polls a
   `BridgeEventSource`, asks the adapter to format each event as Codex turn
   input, sends that input via `turn/start` or `turn/steer`, and advances the
   saved cursor only after app-server acknowledgement.

The CLI shape follows the same boundary:

```bash
cdxm threads --cwd /path/to/project
cdxm monitor watch agmsg --team <team> --name <agent> --cwd /path/to/project
cdxm send --cwd /path/to/project --text "message"
```

`cdxm agmsg watch ...` remains a source-specific shortcut, but the generic surface
is `cdxm monitor watch <adapter> ...`.

## Transport design

### WebSocket transport

`ws://127.0.0.1:<port>` is the primary cross-platform transport. The bridge
must bind or connect only to loopback endpoints in the MVP. Non-loopback
endpoints require a future explicit opt-in because `codex app-server` exposes
auth settings for non-loopback use.

For cdxm-managed sessions, the bridge picks a free loopback port, launches:

```bash
codex app-server --listen ws://127.0.0.1:<port>
```

It waits for `/readyz` before opening the WebSocket. The child process is owned
by the bridge process for foreground commands and for the lifetime of `watch`.

### Unix transport

Unix transport is Unix-only and supports existing macOS App attach. It connects
to a Unix socket path and performs the app-server WebSocket handshake over that
stream. The default App attach path is:

```text
$HOME/.codex/app-server-control/app-server-control.sock
```

This mode never starts, stops, or replaces the Codex App managed daemon. It
only attaches to an existing socket. If the socket is missing, the bridge must
return a clear error and should not create a fake socket path.

### Stdio transport

Stdio transport starts:

```bash
codex app-server --listen stdio://
```

and exchanges newline-delimited JSON messages on stdin/stdout. It is useful for
tests, isolated smoke checks, and simple local process ownership. It is not the
default cross-platform operational mode because loopback WebSocket better
matches the Windows-first target.

## Target realms and endpoint selection

The bridge has two target realms:

- `managed`: cdxm starts or attaches to a cdxm-owned app-server process, normally
  over loopback WebSocket. This is the cross-platform default.
- `app`: cdxm attaches to an existing Codex App managed daemon, normally over
  the macOS Unix socket. This is required when the user wants to interact with
  an existing App UI session.

The CLI should expose a common endpoint selector for every command:

- `--endpoint <url>`: explicit `ws://`, `unix://`, or `stdio://` endpoint.
- `--target managed`: use a cdxm-managed loopback WebSocket app-server.
- `--target app`: use the existing Codex App control socket.

If neither `--endpoint` nor `--target` is supplied, default to `managed` for
portable behavior. The short commands in the MVP are still valid because
common options are optional:

```bash
cdxm threads --cwd <path>
cdxm send --thread <id> --text <msg>
cdxm agmsg watch --team <team> --name <agent> --thread <id>
```

For App UI attach, users can be explicit:

```bash
cdxm --target app threads --cwd <path>
cdxm --target app send --thread <id> --text <msg>
```

## Protocol handshake and methods

Every transport uses the same logical protocol client:

1. Send `initialize` with:

   ```json
   {
     "clientInfo": {
       "name": "codex-monitor",
       "title": "Codex Monitor",
       "version": "0.1.0"
     },
     "capabilities": { "experimentalApi": true }
   }
   ```

2. Send `initialized` notification after a successful initialize response.
3. For thread discovery, call `thread/list`.
4. For target inspection, call `thread/read`.
5. For normal event delivery, call `turn/start`.
6. For explicit raw history append workflows only, call `thread/inject_items`.

The protocol layer should model JSON-RPC style messages:

- Client request: object with `id`, `method`, and `params`.
- Client notification: object with `method` and `params`, no `id`.
- Server response: object with matching `id` plus `result` or `error`.
- Server notification: object with `method` and `params`, no `id`.
- Server request: object with `id`, `method`, and `params`.

The bridge must delete pending requests once a response is received.

## Thread resolution

Target resolution order:

1. `--thread <id>` always wins.
2. `--cwd <path>` resolves through `thread/list` with an exact cwd filter,
   newest-first sort, and limit `20`.
3. If multiple candidates are returned for a cwd, commands that mutate a
   thread must fail with a concise list of candidate thread ids unless the user
   passes an explicit thread id.
4. A future `current App thread` resolver may be added only if the app-server
   exposes a stable current/loaded-thread signal. It is not required for MVP.

`cdxm threads --cwd <path>` is read-only. It prints candidate thread ids,
titles, cwd, created/updated timestamps when available, source kind when
available, and enough metadata for the user to choose an explicit thread.

## Delivery behavior

`turn/start` is the default delivery action. The bridge formats source events
as user input text, preserving source identity and reply instructions in the
prompt body.

The agmsg watch prompt body must include:

```text
agmsg monitor event

Team: <team>
Recipient: <name>
Sender: <sender if known>

<message text>

If this requires a reply, use the agmsg scripts rather than answering only in chat.
```

The bridge processes one delivery per target thread at a time. While a turn is
active, additional source events for the same target are queued. `turn/steer`
is not in the MVP because it changes same-turn semantics and should be added
only after the start/complete path is proven.

If a server request arrives during a turn, the bridge must not approve it. MVP
behavior is concrete: transition the current delivery to
`HumanActionRequired`, print the server request method to stderr, close the
transport, terminate any cdxm-managed child process, and exit non-zero. In
`agmsg watch`, the source event that triggered the server request is not marked
delivered in cdxm state, so a later retry remains possible after human review.

## Source adapter interface

The core source interface should yield normalized events:

```text
BridgeEvent {
  source: string,
  event_id: string,
  observed_at: timestamp,
  title: string,
  body: string,
  cwd_hint: optional absolute path,
  reply_hint: optional structured data,
  metadata: string map
}
```

Adapters must not call Codex APIs directly. They emit `BridgeEvent` values and
let the core handle target selection, delivery, retries, and dedupe.

## agmsg adapter

The agmsg adapter is the first source adapter:

```bash
cdxm agmsg watch --team <team> --name <agent> --thread <id>
```

It must not rely on a `codex` shim, PATH replacement, SessionStart hook,
`inbox.sh`, or `watch.sh`. Those scripts either mark messages read or manage
watcher lifecycle, which is exactly the coupling this bridge is replacing.

MVP reads the agmsg SQLite message store directly. Default DB path:

```text
$HOME/.agents/skills/agmsg/db/messages.db
```

The adapter supports `--agmsg-db <path>` and honors `AGMSG_STORAGE_PATH` by
joining it with `messages.db`. Tests should use `--agmsg-db <fixture>`.

MVP watch semantics:

- Poll SQLite every 2 seconds by default for rows matching
  `team = <team>`, `to_agent = <name>`, and `id > last_seen_id`, ordered by
  ascending `id`.
- Convert each message to one `BridgeEvent`.
- Use `agmsg:<team>:<name>:<id>` as the source event id.
- Persist `last_seen_id` only after Codex delivery reaches a terminal success.
- Do not start or stop existing agmsg watcher processes.
- Do not mutate agmsg role claims or delivery modes.
- Do not reply to agmsg directly; Codex receives the event and may decide to
  reply through the normal agmsg scripts.

## Local state

The bridge stores only local operational state:

- delivered source event ids
- managed app-server process metadata needed for foreground cleanup
- optional last-used endpoint metadata

Use a platform state directory resolved by a Rust directory crate:

- macOS/Linux: XDG state/cache style directory under the user's home
- Windows: local app data directory

MVP state is an inspectable JSON file named `state.json`. It is written through
an atomic temp-file-and-rename sequence. The MVP supports one bridge process per
state file; concurrent writers are rejected with a lock file. This keeps the
first implementation portable without adding a second SQLite dependency beside
agmsg's own store.

## CLI command behavior

### `cdxm threads --cwd <path>`

Read-only command. It initializes the selected target, calls `thread/list` with
the cwd filter, and prints matching threads in newest-first order.

Exit behavior:

- Exit 0 when the request succeeds, even if no threads match.
- Exit non-zero on endpoint connection failure, initialize failure, malformed
  server response, or protocol error.

### `cdxm send --thread <id> --text <msg>`

Mutating command. It initializes the selected target and sends one `turn/start`
to the explicit thread.

Exit behavior:

- Exit 0 after `turn/start` is accepted and the resulting turn reaches a
  terminal completion notification.
- Exit non-zero if the server rejects the request, the transport closes, a
  server request requires human action, or the turn completes with a failure
  status.

### `cdxm agmsg watch --team <team> --name <agent> --thread <id>`

Long-running command. It initializes the selected target, starts the agmsg
adapter, dedupes source events, and sequentially delivers them to the thread.

Exit behavior:

- Runs until interrupted.
- Handles SIGINT/SIGTERM by stopping the source adapter, closing the transport,
  and terminating any cdxm-managed app-server child process.
- Logs delivery failures to stderr with source event ids.
- Does not drop queued events silently; failed events remain undelivered in
  persisted state.

## Safety rules

- Never auto-approve server requests.
- Never connect to non-loopback WebSocket endpoints in MVP unless an explicit
  future unsafe override is added.
- Never log auth tokens, full environment dumps, or raw secret-bearing headers.
- Do not start, stop, or modify the Codex App managed daemon in `--target app`.
- Do not start, stop, or modify existing agmsg watcher processes.
- Default delivery uses `turn/start`, not raw `thread/inject_items`.
- Any live Codex turn smoke test must be explicit in the verifier command and
  should not be hidden inside a unit test.

## Test strategy for implementation

Use fake app-server transports for default automated tests. They should verify:

- initialize and initialized handshake
- thread/list cwd filtering request shape
- thread/read request shape
- turn/start request shape and completion handling
- optional thread/inject_items request shape behind explicit API/flag
- response id cleanup
- server notification classification
- server request refusal behavior
- per-thread delivery queue ordering
- agmsg event dedupe and restart behavior with a fixture SQLite DB

Use real `codex app-server` only for manual or explicitly named integration
checks:

```bash
codex app-server --listen ws://127.0.0.1:<port>
cdxm --endpoint ws://127.0.0.1:<port> threads --cwd /Users/ysk411/dev/codex-monitor
```

Windows verification should at least run:

```bash
cargo check --target x86_64-pc-windows-msvc
```

Unix transport code must be behind `cfg(unix)` so Windows builds do not import
Unix-only modules.

## MVP acceptance criteria

The MVP is complete when current evidence proves all of the following:

- Cargo package is named `codex-monitor`.
- Binaries `codex-monitor` and `cdxm` both call the same library CLI.
- `clientInfo.name` is exactly `codex-monitor`.
- `cdxm threads --cwd <path>` works against a fake app-server and a real
  loopback app-server smoke target.
- `cdxm send --thread <id> --text <msg>` builds the correct `turn/start`
  request and handles completion/failure/server-request paths in tests.
- `cdxm agmsg watch --team <team> --name <agent> --thread <id>` delivers fixture
  agmsg events through the core source adapter interface without PATH shims or
  SessionStart hooks.
- WebSocket, Unix, and stdio transports are represented behind the same
  abstraction, with Unix compiled only on Unix.
- macOS App attach uses the existing control socket without daemon lifecycle
  changes.
- Windows build check passes with the Unix transport excluded.
- No implementation code depends on agmsg outside `sources::agmsg` and CLI
  command wiring for `cdxm agmsg`.

## Implementation plan entry criteria

Before writing the detailed implementation plan, review this spec and confirm
the target realm defaults:

- Default target for short commands is `managed`.
- Existing App UI interaction requires `--target app` or an explicit
  `unix://` endpoint.
- agmsg remains only the first adapter, not the core product identity.

Once confirmed, write the implementation plan in
`docs/superpowers/plans/2026-06-20-codex-monitor-mvp.md`.
