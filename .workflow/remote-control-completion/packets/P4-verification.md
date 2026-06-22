# Packet P4: Verification

Objective: prove the workflow and implementation satisfy the completion contract.

Ownership: command execution and final report.

Do:
- Run workflow verifier.
- Run Rust formatting, tests, and lint.
- Run live non-destructive `cdxm --target app remote doctor` or equivalent.
- Update `final-report.md`.

Do not:
- Change external account or device access.

Expected output: `results/P4-verification.md` and final report.

Verification: all commands and skipped checks are listed with exact status.
