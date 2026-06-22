# Orchestration: codex-monitor MVP implementation

## Execution Rules

- Keep the original objective intact.
- Ask for approval before risky, expensive, external, or destructive actions.
- Keep immediate blocking work local.
- Delegate only bounded, disjoint, materially useful packets.
- Integrate packet results before final verification.

## Branching Rules

- Continue locally while work is confined to source files, tests, docs, and fake
  servers.
- Stop for approval before live mutating Codex thread operations.
- Stop and report exact output if a required verifier repeatedly fails.
- Do not delegate until the user explicitly authorizes worker spawning.

## Packet Prompts

P1: Implement Task 1 from `docs/superpowers/plans/2026-06-20-codex-monitor-mvp.md`. Own scaffold files only. Use TDD naming contract. Verify with `cargo test --test cli_contract` and `cargo fmt --check`.

P2: Implement Task 2 protocol builders and classifiers. Own `src/protocol.rs` and `src/lib.rs`. Verify RED before GREEN.

P3: Implement Task 3 transport trait, memory transport, and client. Own `src/client.rs` and `src/transport/{mod.rs,memory.rs}`.

P4: Implement Task 4 target resolver and CLI command shell. Own `src/target.rs` and `src/cli.rs`.

P5: Implement Task 5 source model, delivery formatting, and JSON state.

P6: Implement Task 6 agmsg SQLite adapter using fixture DB tests.

P7: Implement Task 7 ws, stdio, Unix transports, with Unix code behind `cfg(unix)`.

P8: Implement Task 8 real CLI command dispatch and fake WebSocket app-server tests.

P9: Implement Task 9 agmsg watch loop and shared endpoint opener.

P10: Implement Task 10 README/CI and run final verification.

## Completion Audit

- Confirm `git status --short` is clean or only intentional files remain.
- Confirm all success criteria in `plan.md` have direct command evidence.
- Confirm no live mutating Codex thread operation was run without approval.
