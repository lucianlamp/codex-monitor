# P1 Discovery Result

## Evidence

- `src/cli.rs` has `remote status`, `enable`, `disable`, `pair-start`, `pair-status`, `clients`, `monitor`, `claim`, and `connect`, but no diagnostic command that keeps partial failures.
- `src/client.rs` exposes app-server RPCs for `remoteControl/status/read` and `remoteControl/client/list`.
- `src/remote_control.rs` can read ChatGPT auth, resolve local remote-control enrollment, validate device-key material, claim manual pairing, and connect the remote-control websocket.
- `README.md` documents status, clients, claim, monitor, and connect, but not a combined troubleshooting flow.
- Live non-destructive checks:
  - `cdxm --target app remote status` returned `connected` and an environment id.
  - `cdxm --target app remote clients` returned two clients: an iOS phone and a Macintosh entry.
  - ChatGPT backend `/wham/remote/control/clients?limit=100` returned HTTP 200 with two items.
  - `cdxm --target app remote connect --max-messages 1` failed before backend refresh because local device-key material for the Macintosh entry was unavailable.

## Gap

The current CLI can prove individual surfaces, but it cannot explain the important mixed state:

- phone remote control can still work,
- Codex App settings can show "could not load device list",
- local Mac controller enrollment can be stale or missing its device key,
- backend and app-server lists can still be healthy.

## Implementation Recommendation

Add `remote doctor` as a non-destructive diagnostic command.

It should separately report:

- app-server status
- app-server client list
- ChatGPT backend client list
- local enrollment resolution
- local device-key availability

The command should exit successfully when diagnostics complete, even if individual surfaces are unhealthy, and should print `error` rows for failing surfaces.
