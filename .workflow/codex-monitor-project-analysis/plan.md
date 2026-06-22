# codex-monitor remote-control monitor bridge

## Goal
Implement the next practical remote-control monitor bridge slice: use the
existing Codex App remote-control client enrollment and device-key signer to
refresh a controller session token, connect to the remote-control WebSocket,
answer the backend device-key challenge, and expose this as a safe CLI smoke
path.

## Success Criteria
- Discover the Codex App remote-control WebSocket, refresh, and device-key proof
  contract from local app sources.
- Add a `cdxm remote connect` command that reuses existing auth/global-state
  enrollment and does not print access tokens, session tokens, or signatures.
- Cover refresh/start, refresh/finish, WebSocket challenge validation, and
  proof sending with fake backend/WebSocket E2E.
- Preserve existing `remote claim`, app-server, Unix, and Windows build health.
- Run local verification and a non-destructive live smoke against the current
  Codex App where possible.

## Current Context
- Repository: `/Users/ysk411/dev/codex-monitor`
- Branch: `impl/mvp`
- Existing dirty files include prior remote status/claim work.
- Codex App local source is available via
  `/Applications/Codex.app/Contents/Resources/app.asar`.
- Existing Codex App enrollment is stored under
  `$CODEX_HOME/.codex-global-state.json` or `$HOME/.codex/.codex-global-state.json`.

## Constraints
- No destructive git operations.
- No deploys, external writes, billing, credentials, or production data.
- Do not print secrets, tokens, key material, signatures, or full auth records.
- Keep `~/.codex/plugins/` read-only.
- Keep workflow artifacts under `.workflow/codex-monitor-project-analysis/`.

## Risks
- Remote-control backend contract is private/minified and may drift with Codex
  App updates.
- Device-key signing depends on the installed Codex App native module and the
  local Secure Enclave/Keychain record.
- Full app-server stream proxying is larger than this slice; this slice proves
  authenticated controller WebSocket entry only.
- Live smoke touches the user's ChatGPT remote-control account surface, but is
  non-destructive.

## Approval Required
No extra approval required for local code edits, tests, and the requested
non-destructive E2E smoke. Do not revoke clients, delete keys, deploy, or change
billing/user settings.

## Work Packets
- P1 contract discovery: Codex App asar, native device-key module, WebSocket
  envelope/challenge schemas.
- P2 implementation: Rust auth/enrollment resolution, refresh token flow,
  device-key signing bridge, WebSocket handshake, CLI output.
- P3 tests/docs: fake refresh backend, fake WebSocket challenge, fake signer,
  README command docs.
- P4 verification: unit, fake E2E, full cargo test/fmt/clippy, Windows checks,
  installed binary and live smoke.

## Integration Policy
Prefer local Codex App source and executable tests over memory or README when
they conflict. Keep secrets out of artifacts and final report.

## Verification
- `cargo fmt --check`
- `cargo test remote_control::tests --lib`
- `cargo test remote_connect_completes_device_key_websocket_handshake`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --target x86_64-pc-windows-msvc`
- `cargo check --tests --target x86_64-pc-windows-msvc`
- `cargo install --path . --bin cdxm --force --debug`
- `cdxm --target app remote connect --timeout-ms <bounded>`
- workflow artifact completeness check

## Reusable Artifacts
Keep this run directory. Do not save raw app tokens, pairing codes, signatures,
or remote-control session tokens.
