# Codex Monitor Core Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stabilize Codex Monitor around a generic thread-detection, source-adapter, and delivery core so agmsg/HMSG-style monitors do not need Codex startup shims or SessionStart hooks.

**Architecture:** `target` owns loaded-thread detection, `sources` owns event-source adapters and source-specific turn formatting, and `delivery` owns polling, app-server delivery, and cursor advancement after acknowledgement. The daily CLI surface is `cdxm monitor watch <adapter> ...`; source-specific commands remain source-specific shortcuts.

**Tech Stack:** Rust 2021, clap, tokio, rusqlite for SQLite-backed source adapters, Codex app-server JSON-RPC over stdio/ws/unix transports.

---

### Task 1: Lock The Adapter Boundary

**Files:**
- Modify: `src/sources/mod.rs`
- Modify: `src/sources/agmsg.rs`
- Modify: `src/delivery.rs`
- Test: `tests/agmsg_adapter.rs`

- [x] **Step 1: Keep source polling and formatting behind `BridgeEventSource`**

`BridgeEventSource` must expose `poll_after(last_seen_id)` and `format_event_for_turn(event)`. The default formatter returns the generic title/body form; source adapters override it only when they need source-specific reply instructions.

- [x] **Step 2: Make agmsg the example adapter**

`AgmsgSource` reads matching SQLite rows from `messages`, maps each row to a `BridgeEvent`, uses the SQLite id as `cursor`, and formats events with team, recipient, sender, body, and the agmsg reply instruction.

- [x] **Step 3: Keep delivery source-agnostic**

`delivery::run_monitor_watch` must call `options.source.format_event_for_turn(&event)` and must not branch on `event.source == "agmsg"`.

- [x] **Step 4: Verify adapter tests**

Run:

```bash
cargo test agmsg_adapter
```

Expected: all agmsg adapter tests pass.

### Task 2: Keep Thread Detection Separate

**Files:**
- Modify: `src/target.rs`
- Modify: `src/cli.rs`
- Test: `tests/fake_app_server.rs`

- [ ] **Step 1: Preserve loaded-thread probing**

`resolve_endpoint_and_thread` should keep using `thread/loaded/list` plus `thread/list` for live endpoints. It should return a clear ambiguity error when multiple live endpoints have loaded threads for the same cwd.

- [ ] **Step 2: Preserve the no-resume rule**

Live-target commands must refuse unloaded threads and must not call `thread/resume` implicitly. `thread/resume` remains explicit recovery only.

- [ ] **Step 3: Verify target tests**

Run:

```bash
cargo test threads_command_lists_fake_thread send_resolves_loaded_thread_from_cwd_when_thread_is_omitted
```

Expected: cwd thread resolution and send routing tests pass.

### Task 3: Extend With The Next SQLite Adapter

**Files:**
- Create: `src/sources/hmsg.rs`
- Modify: `src/sources/mod.rs`
- Modify: `src/cli.rs`
- Test: `tests/hmsg_adapter.rs`

- [ ] **Step 1: Mirror the agmsg adapter shape**

Create an `HmsgSource` with fields for DB path and recipient identity. It should implement `BridgeEventSource`, return monotonically ordered `BridgeEvent` records, and use the source row id as `cursor`.

- [ ] **Step 2: Add `cdxm monitor watch hmsg`**

Add a `MonitorWatchCommand::Hmsg` variant that constructs `HmsgSource` and calls `delivery::run_monitor_watch`.

- [ ] **Step 3: Verify HMSG adapter tests**

Run:

```bash
cargo test hmsg_adapter
```

Expected: HMSG fixture DB rows are converted into ordered `BridgeEvent` records and dry-run output shows `source=hmsg`.

### Task 4: Make One-Process Supervision A Separate Layer

**Files:**
- Create: `src/supervisor.rs`
- Modify: `src/lib.rs`
- Modify: `src/cli.rs`
- Test: `tests/supervisor_contract.rs`

- [ ] **Step 1: Define a watch registration model**

Create a registration struct with `source`, `state_key`, `cwd`, optional `thread`, and endpoint selection. Do not mix registration discovery with source adapter parsing.

- [ ] **Step 2: Add a read-only plan command first**

Add a command that prints which registrations would be watched, which endpoint/thread would be used, and which state keys would advance. It must not send turns or update state.

- [ ] **Step 3: Verify supervisor plan tests**

Run:

```bash
cargo test supervisor_contract
```

Expected: planned registrations are printed deterministically and no state file is written.

### Task 5: Final Verification

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-06-20-codex-monitor-design.md`

- [ ] **Step 1: Document the core contract**

README and the design spec should describe thread detection, source adapter, and delivery as separate responsibilities.

- [ ] **Step 2: Run the repo verifier**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: all commands pass.
