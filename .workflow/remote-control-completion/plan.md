# Codex Monitor Remote Control Completion

## Goal
Finish `codex-monitor` as a practical local-first Codex App control bridge with non-destructive remote-control diagnostics for the current Mac.

## Success Criteria
- `cdxm remote doctor` or equivalent reports these surfaces separately:
  - local app-server remote-control status
  - app-server remote-control client list
  - ChatGPT backend remote-control client list
  - local enrolled controller device-key availability
- The command never revokes clients, deletes enrollments, edits Codex App state, or changes external account state.
- Tests cover success, auth/backend failure, app-server failure, and missing device-key diagnostics.
- README explains the monitor bridge workflow and how to interpret the current "settings device list cannot load but phone still works" class of issue.
- Verification passes with Rust tests, formatting, linting, workflow artifact validation, and live non-destructive smoke against the running Codex App.

## Current Context
- Repo: `/Users/ysk411/dev/codex-monitor`.
- Existing implementation is Rust CLI/library with `codex-monitor` primary binary and `cdxm` alias.
- Current dirty tree already contains remote-control work in `src/remote_control.rs`, `src/cli.rs`, `src/client.rs`, `src/protocol.rs`, and tests.
- Live evidence from 2026-06-20/21:
  - `cdxm --target app remote status` returns `connected` with an environment id.
  - `cdxm --target app remote clients` returns an iPhone client and a Mac client.
  - ChatGPT backend `/wham/remote/control/clients` returns HTTP 200 with two items using local auth.
  - `cdxm --target app remote connect` fails for the Mac client because the local device key is missing.
- Codex App settings UI reads both backend browser clients and app-server clients, so a UI load failure can coexist with a live phone connection.

## Constraints
- User-facing conversation must stay Japanese.
- Do not revert existing user or generated changes.
- Do not touch `~/.codex/plugins/` except read-only inspection.
- Do not perform destructive or external account-changing actions without explicit approval.
- Keep remote-control work local-first; do not collapse attach-to-existing App and bridge-managed launch modes.

## Risks
- Remote-control revoke/delete would alter account/device access.
- Local auth files and global state contain sensitive account details.
- Codex App internals are minified and may drift; rely on live checks and robust error reporting.
- Multiple Codex CLI/App sessions are live; avoid stopping or modifying unrelated processes.

## Approval Required
- Required before any revoke, enrollment deletion, keychain/device-key deletion, account sign-out, or Codex App state reset.
- Not required for read-only diagnostics, local code edits, tests, formatting, linting, or non-destructive live status/list calls.

## Work Packets
- P1 Discovery: inspect current code, protocol contracts, tests, README, and live status.
- P2 Implementation: add the diagnostic model and CLI output without destructive behavior.
- P3 Tests/Docs: cover doctor behavior and document usage.
- P4 Verification: run workflow verifier, Rust checks, and live smoke; produce final report.

## Integration Policy
- Accept only findings backed by code references, tests, or live command output.
- Treat App bundle inspection as reference evidence, not a stable public API.
- If live state and tests disagree, keep test fixtures generic and report live-state-specific findings separately.

## Verification
- `python3 /Users/ysk411/.codex/skills/codex-dynamic-workflows/scripts/verify_workflow.py .workflow/remote-control-completion`
- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cdxm --target app remote doctor` or equivalent live non-destructive smoke.

## Reusable Artifacts
- Keep this workflow under `.workflow/remote-control-completion`.
- Add a concise README section for future diagnosis.
- Optionally add `.workflow/recipes/remote-control-doctor.md` only if the implemented diagnostic flow becomes reusable beyond this task.
