# Final Report: remote-control monitor bridge

## Summary

Implemented `cdxm remote connect`, a controller-side remote-control WebSocket
smoke path. It reuses Codex App auth and global-state enrollment, refreshes a
remote-control session token, validates the native device-key record, connects
to `/codex/remote/control/client`, signs the backend device-key challenge, and
prints only non-secret status rows.

## Accepted

- Codex App contract was extracted from the installed app bundle:
  - API base: `https://chatgpt.com/backend-api`
  - refresh start: `/codex/remote/control/client/refresh/start`
  - refresh finish: `/codex/remote/control/client/refresh/finish`
  - WebSocket: `/codex/remote/control/client`
  - protocol header: `x-codex-protocol-version: 3`
  - session header: `x-codex-client-session-token`
- Native device-key signing now matches the App wrapper:
  - payload domain `codex-device-key-sign-payload/v1`
  - normalized payload object
  - signed payload passed as a Buffer to `remote-control-device-key.node`
- Fake E2E proves refresh/start, refresh/finish, WebSocket challenge validation,
  and `device_key_proof` sending.
- README documents `remote connect` and its test/recovery options.

## Rejected

- Full app-server stream proxying was not implemented in this slice.
- No enrollment repair or new key creation was implemented; that requires
  Codex App's step-up authorization path.

## Live Result

`/Users/ysk411/.cargo/bin/cdxm --target app remote status` succeeds and reports
the local Codex App remote-control environment as connected.

`/Users/ysk411/.cargo/bin/cdxm --target app remote connect --timeout-ms 10000`
is blocked by local external state: the existing enrollment record resolves to
client `cli_776f773daa6c4280b50419b51004c9ec`, but the installed native
device-key module returns `device key not found` for that record. The bridge now
detects this before making refresh/WebSocket calls.

## Verification

- `cargo test remote_control::tests --lib`
- `cargo test remote_connect_completes_device_key_websocket_handshake`
- `cargo test`
- `cargo fmt --check`
- `git diff --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --target x86_64-pc-windows-msvc`
- `cargo check --tests --target x86_64-pc-windows-msvc`
- `cargo install --path . --bin cdxm --force --debug`

## Remaining Risks

- The remote-control backend contract is private and can drift with Codex App.
- The current Mac needs Codex App remote-control client re-enrollment before a
  live WebSocket controller session can complete.
- The next code slice is stream proxying: client stream open/close envelopes,
  app-server JSON-RPC forwarding, ack/replay, chunking, ping/pong, and reconnect.
