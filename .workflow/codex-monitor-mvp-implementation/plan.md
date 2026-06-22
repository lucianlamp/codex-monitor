# codex-monitor MVP implementation

## Goal

Implement the MVP described by:

- `docs/superpowers/specs/2026-06-20-codex-monitor-design.md`
- `docs/superpowers/plans/2026-06-20-codex-monitor-mvp.md`

The implementation must produce a Rust package named `codex-monitor`
with primary binary `codex-monitor`, alias binary `cdxm`, and app-server
`clientInfo.name = codex-monitor`.

## Success Criteria

- `cargo test` passes.
- `cargo fmt --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes or any blocker is reported
  with exact output.
- Windows target check `cargo check --target x86_64-pc-windows-msvc` passes or
  missing target/toolchain is reported with exact output.
- CLI commands exist:
  - `cdxm threads --cwd <path>`
  - `cdxm send --thread <id> --text <msg>`
  - `cdxm agmsg watch --team <team> --name <agent> --thread <id>`
- Core modules are source-agnostic; agmsg code is confined to `sources::agmsg`
  and command wiring.
- No Codex PATH shim, SessionStart hook, or existing agmsg watcher lifecycle is
  introduced.

## Current Context

- Branch: `impl/mvp`.
- Existing committed docs only: design spec and MVP implementation plan.
- Current workflow runner is the main Codex session; no delegated workers are
  authorized yet.
- Use TDD for implementation slices: write test, watch it fail, then implement.

## Constraints

- Japanese user-facing progress.
- Keep edits scoped to the plan.
- Do not use destructive git commands.
- Do not run live `cdxm send` against an actual user Codex thread without
  explicit approval.
- Do not create bulky scratch files; keep workflow notes under this directory.

## Risks

- Rust dependency/API drift.
- Unix WebSocket-over-Unix handshake details.
- Windows build insource-specific from Unix-only imports.
- agmsg DB schema assumptions.
- Live Codex App attach can affect a real thread if a mutating command is run.

## Approval Required

- Required before live `cdxm send` against a real thread.
- Required before spawning delegated/parallel workers.
- Required before deleting or rewriting committed history.
- Not required for local code edits, tests, fake app-server tests, or read-only
  app-server schema/help checks.

## Work Packets

- Packet P1: scaffold Cargo package, binaries, README, naming tests.
- Packet P2: app-server protocol builders and classifiers.
- Packet P3: transport trait, memory transport, app-server client.
- Packet P4: target resolver and CLI parser/dispatch shell.
- Packet P5: source event model, delivery formatting, state store.
- Packet P6: agmsg SQLite source adapter.
- Packet P7: ws, stdio, and Unix transports.
- Packet P8: real CLI commands over selected transports and fake app-server.
- Packet P9: agmsg watch delivery loop.
- Packet P10: README, CI workflow, final verification.

## Integration Policy

Implement packets sequentially unless a later packet is read-only and clearly
independent. Commit after each packet when its verifier passes. If a verifier
fails due to the plan being wrong, stop, patch the plan or code deliberately,
and record the decision in `results/`.

## Verification

Narrow checks per packet first. Final checks:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-pc-windows-msvc
python3 /Users/ysk411/.codex/skills/codex-dynamic-workflows/scripts/verify_workflow.py .workflow/codex-monitor-mvp-implementation
```

## Reusable Artifacts

Keep this workflow directory as the reusable orchestration artifact for future
cdxm implementation slices.
