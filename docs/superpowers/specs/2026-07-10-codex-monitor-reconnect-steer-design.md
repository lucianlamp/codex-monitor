# Codex Monitor Reconnect and Steer Delivery Design

**Date:** 2026-07-10  
**Status:** Implemented and accepted by live Windows App restart test

## Context

`cdxm monitor watch agmsg --target app` currently resolves one concrete Codex
App endpoint, opens one app-server connection, and exits on the first delivery
error. Restarting Codex App changes the bridge endpoint. A watcher that remains
attached to an older app-server can then receive `thread not found` and exit,
leaving later agmsg events without a consumer.

Live restart testing exposed a second stale-session case: the previous App
bridge can exit while its child app-server remains alive. That orphan server
continues accepting `thread/read` and `turn/steer`, so transport-error-only
reconnect logic can accept an acknowledgement from the old endpoint, advance
state, and never deliver the input to the newly opened Codex App. Endpoint
`60498` produced this false acknowledgement after the current App moved to
`56473`.

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
8. Reject acknowledgements from a formerly selected dynamic endpoint after the
   logical `app` or `auto` target has moved elsewhere.

## Non-goals

- Forcing `turn/steer` input to render as a separate Codex App bubble.
- Duplicating a steered event with a later `turn/start` solely for display.
- Using `thread/inject_items` as the normal delivery path.
- Restarting Codex App, Codex CLI, or other monitor processes.
- Treating app-server acknowledgement as proof of a specific UI rendering.
- Polling dynamic endpoint discovery continuously when no source event is
  pending.

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

### Dynamic endpoint drift guard

After source polling returns at least one pending event, but before formatting
or sending that event, the watcher re-resolves dynamic logical targets:

- `app` resolves the currently live App bridge endpoint;
- `auto` repeats loaded-thread selection for the requested thread or cwd;
- `explicit` and `managed` keep their existing session without an additional
  probe.

The resolved endpoint and thread are compared with the connected session. If
either differs, or dynamic resolution fails, the watcher closes the old
session and returns to the outer reconnect loop without sending the event and
without advancing state. The next session connects to the newly resolved
target, polls from the unchanged cursor, and retries the same event.

This check runs only when an event is pending. A two-second discovery heartbeat
would add repeated app-server probes while idle without improving delivery
correctness, and directly watching Windows marker files would not cover `auto`
or other platforms.

### Delivery loop

For each source event after the saved cursor:

1. Revalidate a dynamic logical target before any app-server delivery call.
2. If the target drifted, reconnect without sending or advancing state.
3. Format the event once.
4. In `auto` mode, inspect the active turn and call
   `turn_start_or_steer`:
   - active turn: attempt `turn/steer`;
   - no active turn: call `turn/start`;
   - if the active turn ends between inspection and steering, retain the
     existing fallback from failed steer to `turn/start`.
5. On acknowledgement, persist the event cursor atomically.
6. On transport closure, stale endpoint, `thread not found`, oversized-frame
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
6. A still-responsive old endpoint receives no delivery request after dynamic
   target resolution returns a different endpoint; state advances only after
   the replacement endpoint acknowledges the retried event.

The live Windows acceptance test is:

1. Start one `--target app` agmsg watcher pinned to the current thread.
2. Record the watcher PID and current `codex-app-bridge` endpoint.
3. Restart Codex App without stopping the watcher and confirm the App publishes
   a different bridge endpoint while the old app-server may remain alive.
4. While a turn is active, send a unique agmsg message to the watcher identity.
5. Confirm that the running watcher PID survives, rejects the old endpoint,
   resolves the new
   `codex-app-bridge` endpoint, receives a `turn/steer` acknowledgement, and
   advances the agmsg state exactly once.
6. Confirm from the current model turn that the unique message text was
   received. A separate visible App bubble is not required for this active-turn
   case.

Live acceptance completed on 2026-07-10:

- watcher PID `51852`, started before the App restart, remained alive;
- the old endpoint `ws://127.0.0.1:56473` remained responsive while the new
  App bridge published `ws://127.0.0.1:51712`;
- the same watcher PID established its TCP connection to port `51712`;
- doctor resolved thread `019f499f-981b-7613-b5cb-8a4d5cdba90b` on `51712`;
- agmsg event `7356`, token `[cdxm-driftguard-live-20260710-184102]`, reached
  the current model turn; and
- state advanced exactly to `7356` with `pending_after_state_count=0`.

The manually started watcher did not retain stderr to a file, so reconnect was
proved with the surviving process identity, old and new live endpoint
processes, the watcher's established TCP peer, doctor thread resolution, cursor
advancement, and receipt of the unique model-turn token rather than a saved log
line.

## Documentation

README and the installed codex-monitor skill will state:

- active events use `turn/steer` and may not render as separate App bubbles;
- idle events use `turn/start`;
- the watcher reconnects after App restart and does not mark an event delivered
  until acknowledgement; and
- UI visibility is a separate live-smoke property, not implied by state
  advancement.
