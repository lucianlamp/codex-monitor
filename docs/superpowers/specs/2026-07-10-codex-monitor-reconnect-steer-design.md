# Codex Monitor Reconnect and Steer Delivery Design

**Date:** 2026-07-10  
**Status:** Implemented on feature branch

## Context

`cdxm monitor watch agmsg --target app` currently resolves one concrete Codex
App endpoint, opens one app-server connection, and exits on the first delivery
error. Restarting Codex App changes the bridge endpoint. A watcher that remains
attached to an older app-server can then receive `thread not found` and exit,
leaving later agmsg events without a consumer.

The feature branch already:

- rejects marker files whose `bridge_pid` is no longer alive;
- removes the App bridge's inherited 16 MiB WebSocket frame limit; and
- preserves agmsg state until an app-server request is acknowledged.

The remaining work is to make the monitor process survive App endpoint changes
and to apply the same large-message transport configuration to `cdxm` clients.

The user explicitly selected the existing auto-delivery semantics:

- while a turn is active, deliver immediately with `turn/steer`;
- while the thread is idle, deliver with `turn/start`.

An active-turn `turn/steer` input does not have to appear as a separate Codex
App user-message bubble. App-server acknowledgement that the input was appended
to the active turn is the accepted minimum success condition. Idle
`turn/start` delivery remains a normal user turn and can render normally.

This matches the official Codex app-server contract: `turn/steer` appends user
input to an in-flight turn without creating a new turn, while `turn/start`
begins a new turn.

References:

- <https://learn.chatgpt.com/docs/app-server#lifecycle-overview>
- <https://learn.chatgpt.com/docs/app-server#api-overview>

## Goals

1. Keep one watcher process alive across Codex App restarts.
2. Preserve the logical target selector (`app`, `managed`, `auto`, or explicit)
   so a reconnect resolves a fresh concrete endpoint.
3. Preserve auto delivery: active turn to `turn/steer`, idle thread to
   `turn/start`.
4. Advance source state only after the selected app-server operation is
   acknowledged.
5. Retry an unacknowledged event after reconnecting instead of dropping it.
6. Accept local app-server frames and messages larger than tungstenite's
   defaults.
7. Stop cleanly on Ctrl+C without managing unrelated watchers or Codex
   processes.

## Non-goals

- Forcing `turn/steer` input to render as a separate Codex App bubble.
- Duplicating a steered event with a later `turn/start` solely for display.
- Using `thread/inject_items` as the normal delivery path.
- Restarting Codex App, Codex CLI, or other monitor processes.
- Treating app-server acknowledgement as proof of a specific UI rendering.

## Alternatives Considered

### 1. Reconnect inside the Rust monitor loop (chosen)

The watcher retains its logical target and source state, but recreates the
app-server session after endpoint, transport, or delivery failures. This keeps
the same process and pending event alive, works on every supported platform,
and lets state advancement remain coupled to acknowledgement.

### 2. Relaunch the watcher from a Windows shell supervisor

A wrapper could restart `cdxm` after it exits. This is Windows-specific, loses
the in-memory pending-event context, complicates process discovery, and leaves
the core monitor fragile on other platforms.

### 3. Use `turn/start` only

This would favor visible user turns but delay messages while the current turn
is active. The user explicitly requires `turn/steer` during active turns, so
this approach is rejected.

## Architecture

### Logical target and reconnectable session

`run_monitor_watch` keeps an immutable copy of the requested logical endpoint,
thread id, and cwd. A session setup function performs these steps each time:

1. Resolve the logical endpoint to the current concrete endpoint.
2. Resolve or validate the pinned thread on that endpoint.
3. Open the transport.
4. Send `initialize` and `initialized`.
5. Confirm that the target thread is loaded without calling `thread/resume`.

If setup fails, the watcher logs one concise reconnect message, waits two
seconds, and retries. It does not advance source state.

### Delivery loop

For each source event after the saved cursor:

1. Format the event once.
2. In `auto` mode, inspect the active turn and call
   `turn_start_or_steer`:
   - active turn: attempt `turn/steer`;
   - no active turn: call `turn/start`;
   - if the active turn ends between inspection and steering, retain the
     existing fallback from failed steer to `turn/start`.
3. On acknowledgement, persist the event cursor atomically.
4. On transport closure, stale endpoint, `thread not found`, oversized-frame
   error, or another app-server delivery failure, close the session, reconnect,
   and retry the same event.

Source polling failures and state persistence failures remain fatal. A source
failure means the event stream is unavailable; a state persistence failure
occurs after a delivery may already have been accepted, so immediate blind
retry could duplicate the event.

### Large local WebSocket messages

`WsTransport` uses `connect_async_with_config` with both
`max_frame_size(None)` and `max_message_size(None)`, matching the App bridge.
All accepted endpoints are loopback-only, and Codex App/app-server remain the
authoritative payload producers and consumers.

### Shutdown

Ctrl+C interrupts both the connected polling loop and reconnect backoff. The
watcher closes the current transport when possible and exits zero. It does not
stop or replace other watcher, App, CLI, or app-server processes.

## Testing

Automated coverage must prove:

1. The existing auto-delivery tests still choose `turn/steer` for an active
   turn and `turn/start` for an idle thread.
2. A WebSocket response containing a single frame larger than 16 MiB passes
   through `WsTransport`.
3. A first session that fails with a stale-thread or transport error does not
   advance state; a second session retries the same event and advances state
   exactly once after acknowledgement.
4. Ctrl+C can end reconnect backoff without waiting for another successful
   connection.
5. Existing dry-run behavior remains one-shot and non-mutating.

The live Windows acceptance test is:

1. Start one `--target app` agmsg watcher pinned to the current thread.
2. Restart Codex App without stopping the watcher.
3. While a turn is active, send a unique agmsg message to the watcher identity.
4. Confirm that the running watcher PID survives, resolves the new
   `codex-app-bridge` endpoint, receives a `turn/steer` acknowledgement, and
   advances the agmsg state exactly once.
5. Confirm from the current model turn that the unique message text was
   received. A separate visible App bubble is not required for this active-turn
   case.

## Documentation

README and the installed codex-monitor skill will state:

- active events use `turn/steer` and may not render as separate App bubbles;
- idle events use `turn/start`;
- the watcher reconnects after App restart and does not mark an event delivered
  until acknowledgement; and
- UI visibility is a separate live-smoke property, not implied by state
  advancement.
