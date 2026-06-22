# Orchestration: Codex Monitor Remote Control Completion

## Execution Rules

- Keep the original objective intact.
- Ask for approval before risky, expensive, external, or destructive actions.
- Keep immediate blocking work local.
- Delegate only bounded, disjoint, materially useful packets.
- Integrate packet results before final verification.
- Because no separate writable subagent runner is currently required, simulate packets as isolated local passes and save concise result notes.

## Branching Rules
- If existing tests already cover a needed protocol field, extend those tests instead of adding a parallel fixture.
- If live Codex App is unavailable, keep implementation test-complete and mark live smoke as skipped with exact command/error.
- If diagnosis finds a destructive remediation, document it as a gated next action rather than executing it.

## Packet Prompts
- P1 Discovery: Read current Rust remote-control code, CLI command shape, protocol requests, tests, README, and live non-destructive command outputs. Write `results/P1-discovery.md`.
- P2 Implementation: Add a non-destructive doctor command/model that separates app-server status, app-server clients, backend clients, and local device-key availability. Write `results/P2-implementation.md`.
- P3 Tests/Docs: Add focused tests and README usage. Write `results/P3-tests-docs.md`.
- P4 Verification: Run workflow and Rust checks plus live smoke. Write `results/P4-verification.md`.

## Completion Audit
- All success criteria in `plan.md` have evidence.
- No external/device access was revoked or deleted.
- Final report lists accepted results, rejected ideas, conflicts, verification, and remaining risks.
