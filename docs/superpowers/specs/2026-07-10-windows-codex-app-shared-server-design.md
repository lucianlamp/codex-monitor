# Windows Codex App Shared Server Design

## Goal

Deliver `agmsg` events into the exact Codex App thread currently visible on
Windows. Delivery must not create, resume, or select a different thread.

## Confirmed Failure

Codex App launches its bundled `codex.exe app-server` as a child and speaks
JSONL over the child's standard input and output. A separately discovered
loopback WebSocket app-server belongs to a Codex CLI session, even when the
binary path contains `OpenAI\Codex`. Process-name and listening-port discovery
therefore cannot prove that an endpoint belongs to Codex App.

The current `--target app` Windows implementation can consequently select the
wrong app-server. A successful request or acknowledgement against that endpoint
does not prove delivery to the visible Codex App thread.

## Selected Architecture

Add a Windows Codex App bridge executable and install it as the App's Codex CLI
override through `CODEX_CLI_PATH`.

When Codex App invokes the bridge with an `app-server` command, the bridge:

1. Locates the real Codex executable without consulting `CODEX_CLI_PATH`.
2. Chooses an unused loopback TCP port.
3. Starts the real app-server with the original config arguments and
   `--listen ws://127.0.0.1:<port>`.
4. Connects one WebSocket client for Codex App.
5. Proxies each JSONL request from App stdin to one WebSocket text frame and
   each WebSocket response or notification to one stdout JSON line.
6. Atomically publishes a per-bridge marker containing the endpoint, bridge PID, server
   PID, real executable path, and format version.
7. Removes the marker and terminates the child server when App closes the
   bridge.

The app-server remains one process with multiple clients. Codex App and `cdxm`
therefore observe the same loaded thread manager and the same active turns.

For any invocation other than `app-server`, the bridge runs the real Codex
executable with unchanged arguments and returns its exit code.

## Components

### `cdxm-codex-app-bridge`

A new Rust binary owns process launch, JSONL/WebSocket proxying, lifecycle, and
marker publication. It does not implement app-server protocol methods and does
not inspect message bodies beyond validating that stdin lines are JSON before
forwarding them.

The real Codex executable is resolved in this order:

1. `CDXM_REAL_CODEX` when explicitly configured by the installer.
2. The Codex App resources directory supplied by the Electron process, when
   available.
3. The known per-user Codex binary under `%LOCALAPPDATA%\OpenAI\Codex\bin`.

The bridge rejects a path resolving back to itself.

### App endpoint marker

Markers live below the existing codex-monitor runtime directory in the
user-local application-data location and are named by bridge PID. Each marker
is written by replace-rename so readers never see a partial document.

`cdxm --target app` on Windows reads only these markers. It validates the
format, bridge/server liveness, loopback URL, and endpoint reachability. It no
longer classifies arbitrary `codex.exe app-server` processes as Codex App.

`cdxm targets` may continue to list other CLI endpoints for `auto`, but their
source remains distinct from the marker-backed `codex-app-bridge` endpoint.

### Installer integration

`install.ps1` gains an explicit Codex App bridge option. Enabling it:

- installs all three binaries;
- records the real Codex path;
- copies that executable into the codex-monitor runtime because WindowsApps
  package executables cannot be launched directly by the external bridge;
- preserves any prior user-level `CODEX_CLI_PATH` value;
- sets user-level `CODEX_CLI_PATH` to the bridge;
- prints that Codex App must be restarted.

Removal restores the preserved value and removes only codex-monitor-owned
bridge configuration. It does not modify files in the packaged Windows App.

The existing CLI shim option remains independent.

## Error Handling

- If the real executable is missing or recursive, the bridge exits before
  publishing a marker and prints a concise diagnostic to stderr.
- If the child server fails readiness, the bridge terminates it and leaves no
  marker.
- If App stdin, WebSocket, or child process closes, the bridge closes the other
  sides and removes its marker.
- A stale or malformed marker is ignored with a diagnostic; `--target app`
  never falls back to an arbitrary process endpoint.
- Multiple App bridge markers are rejected as ambiguous unless the caller
  supplies an explicit endpoint.

## Security

- The app-server listens only on `127.0.0.1`.
- Existing WebSocket Origin rejection remains in force.
- No token, account credential, message content, or device key is stored in the
  marker.
- The bridge does not enable remote access and does not change Codex approval
  or sandbox settings.

## Verification

Automated coverage must prove:

- app-server argument interception and non-app-server passthrough;
- real executable recursion rejection;
- marker atomicity, validation, stale cleanup, and ambiguity errors;
- bidirectional JSONL/WebSocket forwarding with server requests and
  notifications;
- Windows installer enable, idempotence, and restore behavior;
- `--target app` refuses ordinary CLI app-server candidates.

Repository verification is `cargo fmt --check`, `cargo test`,
`cargo clippy --all-targets -- -D warnings`, and `git diff --check`.

Live acceptance requires all of the following after restarting Codex App:

1. `cdxm targets` labels the marker endpoint `codex-app-bridge`.
2. `cdxm --target app loaded` includes this visible thread.
3. `cdxm --target app threads --cwd <repo>` resolves this visible thread.
4. The `cdxm/codex` watcher starts pinned to that thread.
5. A new `agmsg` message is acknowledged and visibly appears in this same
   Codex App screen without opening or forking another thread.

Passing unit tests or receiving an app-server acknowledgement alone is not
sufficient acceptance evidence.

## Rollback

Disable the installer integration, restore the prior `CODEX_CLI_PATH`, and
restart Codex App. The packaged Codex App and its bundled executable remain
untouched throughout installation and rollback.
