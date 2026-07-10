# Dynamic Endpoint Drift Guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent `app` and `auto` monitor watchers from accepting delivery acknowledgements from an old but still-responsive app-server after the logical target moves.

**Architecture:** Poll the source first. When at least one event is pending, re-resolve only dynamic logical targets and classify the connected session as current or drifted before any app-server delivery call. A drifted session closes without sending or advancing state; the existing outer session loop reconnects and polls the same event again.

**Tech Stack:** Rust, Tokio, existing `AppServerClient`, `BridgeEventSource`, `MemoryTransport`, and fake app-server integration tests.

---

### Task 1: Guard pending delivery against endpoint drift

**Files:**
- Modify: `src/delivery.rs`
- Test: `src/delivery.rs`

- [ ] **Step 1: Add failing drift classification and no-send tests**

Add a `dynamic_target_drift_is_detected` test that classifies an `Endpoint::App` session connected to `ws://127.0.0.1:60498` against a freshly resolved `ws://127.0.0.1:56473` target and expects `TargetGuard::Drifted`.

Add a `drifted_target_does_not_send_or_advance_state` async test. Build one fixed event, a `MemoryTransport` containing responses that would acknowledge a steer, and call the delivery helper with:

```rust
TargetGuard::Drifted {
    endpoint: crate::target::Endpoint::Explicit(
        "ws://127.0.0.1:56473".into(),
    ),
    thread: "thread-1".into(),
}
```

Assert all three invariants:

```rust
assert!(matches!(pass, super::DeliveryPass::TargetDrifted { .. }));
assert_eq!(state.last_seen("agmsg:dev:codex"), 0);
assert!(client.into_inner().sent.is_empty());
```

- [ ] **Step 2: Run the new tests and verify red**

Run:

```powershell
cargo test delivery::tests::dynamic_target_drift_is_detected -- --exact
cargo test delivery::tests::drifted_target_does_not_send_or_advance_state -- --exact
```

Expected: compilation fails because `TargetGuard`, `DeliveryPass::TargetDrifted`, and the guarded delivery signature do not exist yet.

- [ ] **Step 3: Implement target classification**

Add these private types to `src/delivery.rs`:

```rust
enum TargetGuard {
    Current,
    Drifted {
        endpoint: crate::target::Endpoint,
        thread: String,
    },
}

enum DeliveryPass {
    Healthy,
    TargetDrifted {
        endpoint: crate::target::Endpoint,
        thread: String,
    },
    SessionFailed {
        event_id: String,
        error: anyhow::Error,
    },
}
```

Add `classify_resolved_target` so only `Endpoint::App` and `Endpoint::Auto` can drift. Static `Explicit` and `Managed` targets always return `TargetGuard::Current`. For dynamic targets, return `Current` only when both endpoint and thread still match the connected session.

Add `revalidate_dynamic_target`:

```rust
async fn revalidate_dynamic_target(
    logical_endpoint: &crate::target::Endpoint,
    requested_thread: &Option<String>,
    cwd: &Option<std::path::PathBuf>,
    connected_endpoint: &crate::target::Endpoint,
    connected_thread: &str,
) -> anyhow::Result<TargetGuard>
```

Return `Current` immediately for static targets. Otherwise call `resolve_endpoint_and_thread` with cloned logical inputs and classify the fresh result.

- [ ] **Step 4: Refactor the delivery helper around already-polled events**

Rename `deliver_available_events` to `deliver_polled_events`. Replace its source polling with `events: Vec<BridgeEvent>` and add `guard: TargetGuard`. Return `DeliveryPass::TargetDrifted` before formatting or sending any event when the guard is drifted. Keep state persistence after acknowledgement only.

Move `source.poll_after(state.last_seen(...))` into the connected loop in `run_monitor_watch`. Source errors remain fatal. When events are non-empty, call `revalidate_dynamic_target` before `deliver_polled_events`; when events are empty, use `TargetGuard::Current` without discovery.

Handle `DeliveryPass::TargetDrifted` by logging the connected and replacement endpoint, closing the old client, and immediately continuing the outer session loop. Handle revalidation errors like reconnectable session failures, with the existing shutdown-aware two-second delay and no state advancement.

- [ ] **Step 5: Run focused and existing reconnect tests**

Run:

```powershell
cargo test delivery::tests
cargo test --test fake_app_server monitor_watch_reconnects_and_retries_unacknowledged_event -- --exact
cargo test client::tests
```

Expected: all tests pass, including the original error-driven reconnect test and the new still-responsive-old-endpoint guard tests.

- [ ] **Step 6: Commit the implementation**

```powershell
git add src/delivery.rs
git commit -m "fix: reject stale dynamic endpoints"
```

### Task 2: Document, verify, and prepare the corrected watcher

**Files:**
- Modify: `README.md`
- Modify: `skills/codex-monitor/SKILL.md`
- Modify: `docs/superpowers/specs/2026-07-10-codex-monitor-reconnect-steer-design.md`

- [ ] **Step 1: Update behavior documentation**

Document that pending events on `app` and `auto` revalidate the logical target before delivery, that a still-responsive stale endpoint is rejected, and that `explicit`/`managed` do not add discovery probes. Change the design status to implemented only after tests pass.

- [ ] **Step 2: Run complete verification**

Run:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --bins
git diff --check
```

Expected: every command exits zero; the release directory contains `cdxm.exe`, `codex-monitor.exe`, and `cdxm-codex-app-bridge.exe`.

- [ ] **Step 3: Commit documentation and push the draft PR**

```powershell
git add README.md skills/codex-monitor/SKILL.md docs/superpowers/specs/2026-07-10-codex-monitor-reconnect-steer-design.md
git commit -m "docs: explain dynamic endpoint revalidation"
git push origin HEAD:codex/windows-codex-app-target
```

Keep PR #7 in Draft until live restart acceptance succeeds.

- [ ] **Step 4: Install and replace only the authorized watcher**

Transactionally replace the installed `cdxm.exe` and `codex-monitor.exe` with the verified release binaries, copy the updated skill, then stop only the current `cdxm/codex` watcher and restart it with the same `--target app`, team, name, cwd, mode, and thread arguments. Do not stop unrelated watcher processes.

- [ ] **Step 5: Repeat live restart acceptance**

Record the new watcher PID and App bridge endpoint, ask the user to restart Codex App, then send a unique self-addressed agmsg event through `send.sh`. Verify:

```text
same watcher PID
new codex-app-bridge endpoint
log records target drift and reconnection
doctor state equals the new agmsg id
pending_after_state_count=0
unique token reaches the current model turn
```

Only after these checks may the drift fix be considered complete and PR #7 be marked Ready.
