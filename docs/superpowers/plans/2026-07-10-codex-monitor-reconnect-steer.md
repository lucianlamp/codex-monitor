# Codex Monitor Reconnect and Steer Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep an agmsg watcher alive across Codex App restarts, retry the same unacknowledged event after reconnecting, steer active turns, and accept app-server WebSocket messages larger than 16 MiB.

**Architecture:** Treat `Endpoint` as a logical target and resolve it afresh whenever the app-server session fails. Keep cursor advancement behind a successful app-server acknowledgement so polling the source after reconnect naturally returns the same event. Configure both direct and managed WebSocket connections with unlimited tungstenite frame/message limits.

**Tech Stack:** Rust, Tokio, tokio-tungstenite, clap, rusqlite, existing in-memory app-server transport tests.

---

### Task 1: Accept large app-server WebSocket messages

**Files:**
- Modify: `src/transport/ws.rs`
- Test: `src/transport/ws.rs`

1. Add an async loopback WebSocket test whose server sends a valid JSON text message larger than 16 MiB and assert that `WsTransport::recv()` returns the whole message.
2. Run `cargo test transport::ws::tests::receives_message_larger_than_default_limit -- --exact`; confirm the test fails with tungstenite's space-limit error.
3. Add one shared `WebSocketConfig` builder with `max_frame_size(None)` and `max_message_size(None)`, and use `connect_async_with_config(..., false)` from both `connect` and `start_managed`.
4. Re-run the targeted test and `cargo test transport::ws`; commit as `fix: accept large app-server messages`.

### Task 2: Preserve unacknowledged events across reconnects

**Files:**
- Modify: `src/delivery.rs`
- Test: `src/delivery.rs`

1. Extract one delivery-pass helper that polls strictly after the persisted cursor, formats each event, calls `turn/start` or `turn/steer`, and saves the cursor only after an acknowledgement.
2. Add a test using a fixed source and `MemoryTransport`: the first steer response fails and leaves the cursor unchanged; a second client acknowledges the same event and advances the cursor exactly once.
3. Run the targeted test and confirm it fails before the refactor.
4. Implement the helper without adding a separate pending-event store; the source plus unchanged cursor is the retry queue.
5. Re-run the targeted test and existing delivery/client tests.

### Task 3: Reconnect to the logical target without ending the watcher

**Files:**
- Modify: `src/delivery.rs`
- Test: `src/delivery.rs`

1. Move endpoint resolution, connection, initialization, thread selection, and loaded-thread validation into a reconnectable session-opening helper.
2. Wrap live monitoring in an outer session loop. A connection/setup/delivery error logs the failure, closes the failed client when available, waits with shutdown awareness, and resolves the original logical endpoint again. Source polling and state persistence errors remain fatal.
3. Keep `DeliveryMode::Auto` behavior: steer an in-progress turn, otherwise start a turn. An acknowledged steer is sufficient even if the App UI does not render a separate bubble.
4. Add focused tests for retry classification and shutdown-aware backoff where practical, then run `cargo test delivery client`.
5. Commit as `fix: reconnect monitor after app restarts`.

### Task 4: Document and verify the completed behavior

**Files:**
- Modify: `README.md`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify if needed: `docs/superpowers/specs/2026-07-10-codex-monitor-reconnect-steer-design.md`

1. Document that live watchers reconnect after Codex App replacement, retry unacknowledged events, and use steer for active turns without promising a separate UI bubble.
2. Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features`.
3. Review `git diff --check` and the full branch diff for scope, dead code, and cursor-safety regressions.
4. Commit as `docs: explain resilient app delivery` and push the existing PR branch.
5. Build the release binaries and run a live temporary agmsg watcher test: deliver once, restart Codex App, deliver another unique token, verify the same watcher PID survives and each cursor advances once. Do not replace unrelated watchers.
