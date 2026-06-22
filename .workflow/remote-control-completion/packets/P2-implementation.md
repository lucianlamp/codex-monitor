# Packet P2: Implementation

Objective: implement non-destructive remote-control diagnostics.

Ownership: Rust implementation and CLI surfaces needed for doctor behavior.

Do:
- Add a diagnostic result model that keeps partial failures.
- Add CLI output that separates app-server status, app-server clients, backend clients, and local device-key checks.
- Preserve existing command behavior.

Do not:
- Add destructive remediation commands.
- Hide failures behind a single generic error.
- Depend on minified App bundle internals at runtime.

Expected output: code changes plus `results/P2-implementation.md`.

Verification: focused Rust tests for partial success/failure behavior.
