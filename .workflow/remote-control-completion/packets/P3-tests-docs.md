# Packet P3: Tests And Docs

Objective: make the new diagnostic behavior understandable and regression-tested.

Ownership: test fixtures, CLI contract tests, README.

Do:
- Add tests for doctor output and backend/device-key failure classification.
- Update README with the monitor bridge and settings-list troubleshooting flow.
- Keep examples non-destructive.

Do not:
- Include secrets, tokens, or account-private details.
- Document revoke/delete as an automatic fix.

Expected output: tests/docs plus `results/P3-tests-docs.md`.

Verification: `cargo test` and README examples matching implemented CLI.
