# Packet P1: Discovery

Objective: inspect the current implementation, tests, docs, and live non-destructive remote-control state.

Ownership: read-only except writing `results/P1-discovery.md`.

Do:
- Inspect `src/remote_control.rs`, `src/cli.rs`, `src/client.rs`, `src/protocol.rs`, `tests/fake_app_server.rs`, `README.md`.
- Run non-destructive status/list/backend checks.
- Identify exact gaps for a `remote doctor` diagnostic.

Do not:
- Revoke clients.
- Delete enrollments or keys.
- Reset or sign out Codex App.

Expected output: concise evidence and implementation recommendations in `results/P1-discovery.md`.

Verification: every recommendation must cite a file/function or command result.
