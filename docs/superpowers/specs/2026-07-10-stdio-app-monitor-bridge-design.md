# Stdio-Preserving Codex App Monitor Bridge Design

## Problem

The current Windows App bridge replaces the Codex App's native stdio
app-server transport with a loopback WebSocket app-server. That makes the App
thread observable by `cdxm`, but a live A/B test showed that it also changes
Browser Use behavior:

- with `cdxm-codex-app-bridge.exe`, navigation to Google and localhost is
  rejected as an enterprise-network-policy block;
- with the App's native packaged Codex app-server, Google loads and the in-app
  Playwright API returns the page DOM.

The bridge must therefore preserve the native stdio transport while exposing
only the small monitoring surface needed for agmsg delivery.

## Goals

- Preserve the Codex App's native stdio app-server connection and Browser Use
  behavior.
- Expose the current App thread to `cdxm` through a loopback-only monitor
  endpoint.
- Support the minimum protocol required to locate a loaded thread and deliver
  `turn/start` or `turn/steer` input.
- Keep the existing npm-backed Codex CLI monitor path unchanged.
- Scope cleanup to child PIDs owned by the bridge.

## Non-goals

- Do not implement a general multi-client app-server proxy.
- Do not expose remote-control, account, authentication, approval, tool,
  download, upload, or MCP methods through the monitor endpoint.
- Do not alter the Codex CLI shim or its current WebSocket app-server path.
- Do not terminate stale or unrelated Codex processes by executable name.

## Selected Architecture

### Native App path

When the App invokes `cdxm-codex-app-bridge.exe ... app-server`, the bridge
launches the matching real Codex executable with the original arguments and
without adding `--listen`. The child therefore uses its normal stdin/stdout
JSONL transport.

The bridge forwards App stdin to child stdin and child stdout to App stdout.
Messages originating from the App remain byte-equivalent JSON values; the
bridge does not reinterpret or rewrite their IDs.

### Monitor endpoint

In parallel, the bridge binds a WebSocket listener on `127.0.0.1:0` and writes
the existing App-target marker with the listener endpoint, bridge PID, child
PID, and real Codex path. `cdxm --target app` continues to discover the marker
as a `codex-app-bridge` target.

The endpoint implements only these client methods:

- `initialize`;
- `initialized`;
- `thread/list`;
- `thread/read`;
- `thread/loaded/list`;
- `turn/start`;
- `turn/steer`.

The bridge answers `initialize` locally and consumes `initialized`. Every other
allowed request is forwarded only after the App has completed its own
`initialize`/`initialized` handshake. Requests arriving earlier receive a
retryable server-not-ready error. Unknown methods,
notifications other than `initialized`, and messages with invalid request
shapes receive an explicit JSON-RPC error and are not forwarded.

### Request routing

App requests pass through unchanged. Monitor requests receive an internal
string ID containing a random per-bridge nonce, connection number, and request
sequence. The bridge records the connection and original numeric ID,
serializes all App and monitor writes through one child-stdin writer, and
restores the original ID on the matching response.

Child output is routed as follows:

- a response matching an internal monitor ID goes only to that monitor client;
- any other response or server request goes only to the App;
- notifications always go to the App;
- notifications needed by current `cdxm` turn handling are also broadcast to
  connected monitor clients.

The monitor notification allowlist is `turn/started`, `turn/completed`, and
`error`. Other notifications remain App-only. A disconnected monitor client's
pending IDs are retired; late responses for those IDs are discarded rather
than leaked into the App connection.

## Components

### `StdioAppServer`

Owns the real Codex child process and its piped stdin/stdout. It starts only the
PID created by the bridge and stops only that owned PID when the App connection
ends or startup fails.

### `MonitorRouter`

Validates the method allowlist, allocates internal IDs, tracks pending response
ownership, restores client IDs, and selects which notifications are broadcast.
Routing logic is independent of sockets so it can be unit tested with JSON
values.

### `MonitorListener`

Accepts loopback WebSocket connections and connects them to `MonitorRouter`.
Slow or disconnected monitor clients must not block the App-to-child stdio
path; a bounded per-client output queue closes the slow monitor connection when
full.

### Existing target marker

The marker format and Windows target discovery remain unchanged. The endpoint
now identifies the bridge's minimal monitor listener rather than a native
app-server WebSocket listener.

## Lifecycle and Failure Handling

1. Bind the loopback monitor listener.
2. Launch the real Codex app-server in stdio mode with piped stdin/stdout and
   inherited stderr.
3. Begin the transparent App/child stdio proxy and wait for the App's
   `initialized` notification.
4. Publish the App-target marker only after the child session is initialized.
5. Run App input, child output, monitor listener, and child-exit handling
   concurrently.
6. On App stdin EOF, close monitor clients, close child stdin, wait briefly for
   normal child exit, then terminate only that child PID if necessary.
7. Remove the marker on every bridge exit path.

If the child exits unexpectedly, the bridge reports the child status to stderr
and exits non-zero. A malformed App JSONL line is forwarded unchanged because
the App owns that transport. A malformed monitor message is rejected locally.

## Security Boundary

- The monitor listener accepts only loopback connections.
- The method allowlist prevents account, approval, tool, MCP, filesystem, and
  remote-control operations from crossing the monitor endpoint.
- Internal IDs are bridge-generated and cannot be selected by clients.
- The bridge does not expose child server requests to monitor clients.
- No cleanup command selects processes by name or wildcard.

## Codex CLI Behavior

The CLI path remains unchanged: the public shim resolves npm Codex `0.144.1`,
starts or reuses its WebSocket app-server, and connects the interactive TUI with
`--remote`. The stdio-preserving bridge is used only when the Codex App invokes
the configured `CODEX_CLI_PATH`.

## Verification

Automated coverage must include:

- original App requests and responses remain unchanged;
- monitor initialize is handled locally;
- every allowed monitor method is forwarded with a unique internal ID;
- disallowed methods and malformed messages are rejected without child writes;
- monitor responses restore the original ID and never reach the App;
- server requests remain App-only;
- turn notifications reach both App and monitor clients;
- slow or disconnected monitor clients cannot stall App stdio;
- frames larger than the former WebSocket limits remain supported;
- marker cleanup and child cleanup affect only bridge-owned resources.

Live Windows acceptance requires all of the following in the same installed
configuration:

1. Codex App starts through `cdxm-codex-app-bridge.exe` and the real child has
   no `--listen ws://...` argument.
2. In-app Browser navigation to `https://www.google.com/` succeeds and
   Playwright reads the Google DOM.
3. `cdxm --target app loaded` includes the visible App thread.
4. A unique agmsg event is acknowledged by the intended thread through
   `turn/start` or `turn/steer`.
5. A new terminal `codex --version` still reports npm Codex `0.144.1` and its
   monitor endpoint remains usable.

## Rollout and Rollback

Keep the App on its native packaged runtime while implementation and automated
tests are in progress. After installing the rebuilt bridge, re-enable the
owned `CODEX_CLI_PATH`/`CDXM_REAL_CODEX` values and restart only Codex App for
live acceptance. If Browser or delivery fails, clear those two user variables
again and restart App; the packaged native path remains the safe rollback.
