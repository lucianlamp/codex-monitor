# Orchestration: remote-control monitor bridge

## Execution Rules

- Keep the objective focused on controller WebSocket entry, not full stream
  proxying.
- Ask before destructive or account-mutating operations such as revoking clients
  or deleting device keys.
- Keep immediate implementation local; no subagents are required for this slice.
- Do not record raw tokens, pairing codes, signatures, or private key material.

## Branching Rules

- If Codex App source and README disagree, treat the installed Codex App source
  and executable fake tests as authority.
- If the native device-key module cannot be loaded, keep the CLI error explicit
  and do not create a replacement enrollment.
- If live backend smoke fails after local tests pass, report the exact stage:
  auth refresh, enrollment resolution, refresh/start, refresh/finish,
  WebSocket connect, or device-key challenge.

## Packet Prompts

- P1: Extract remote-control refresh, WebSocket, and device-key proof contracts
  from the installed Codex App bundle.
- P2: Implement Rust helpers and `cdxm remote connect` for refresh plus
  WebSocket device-key challenge completion.
- P3: Add fake backend/WebSocket E2E, unit tests, and README docs.
- P4: Run local verification, install the binary, and run bounded live smoke.

## Completion Audit

- CLI has `remote connect`.
- Fake E2E proves refresh/start, refresh/finish, WebSocket challenge validation,
  and proof sending.
- Full verification list in `plan.md` is run or skipped with reason.
- Final report separates completed controller handshake from remaining full
  stream proxy work.
