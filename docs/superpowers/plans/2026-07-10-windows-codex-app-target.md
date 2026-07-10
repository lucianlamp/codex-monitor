# Windows Codex App Target Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cdxm --target app` connect to the live Codex App endpoint on Windows instead of rejecting the platform.

**Architecture:** Keep the existing Unix control-socket behavior. On Windows, resolve `Endpoint::App` to the single live loopback WebSocket endpoint discovered from a Codex app-server process; return an actionable error when none or several are available. Resolve the named target before thread probing or transport creation.

**Tech Stack:** Rust, Tokio, existing Windows process and TCP inventory parser, Cargo tests.

---

### Task 1: Resolve the Windows App Target

**Files:**
- Modify: `src/target.rs`
- Modify: `src/cli.rs`

- [x] **Step 1: Write failing target-resolution tests**

Add tests proving that Windows app-target candidate selection accepts one `codex-app-server-process`, rejects no candidates, and rejects ambiguity.

- [x] **Step 2: Run the focused tests and verify failure**

Run: `cargo test windows_app_endpoint`

Expected: compilation or assertion failure because app-target selection does not exist.

- [x] **Step 3: Implement minimal endpoint selection**

Add a pure candidate-selection helper, use it from Windows `Endpoint::App` resolution, and resolve the named endpoint at the beginning of `resolve_endpoint_and_thread`.

- [x] **Step 4: Run focused tests**

Run: `cargo test windows_app_endpoint`

Expected: all focused tests pass.

### Task 2: Verify and Install

**Files:**
- Modify if required: `README.md`

- [x] **Step 1: Update the platform description**

Document that `--target app` uses the Unix control socket on Unix and live Codex App loopback endpoint discovery on Windows.

- [x] **Step 2: Run repository verification**

Run: `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `git diff --check`.

Expected: every command exits successfully with zero failing tests or warnings.

- [x] **Step 3: Install the verified binaries**

Run: `powershell -ExecutionPolicy Bypass -File .\install.ps1`.

Expected: `cdxm.exe` and `codex-monitor.exe` are installed successfully.

- [x] **Step 4: Verify against the running Codex App**

Run: `~/.codex-monitor/bin/cdxm.exe loaded --target app` and `~/.codex-monitor/bin/cdxm.exe loaded --target app --cwd <workspace>` as applicable.

Expected: the command reaches the Windows Codex App endpoint and no longer reports that `--target app` requires Unix socket support.
