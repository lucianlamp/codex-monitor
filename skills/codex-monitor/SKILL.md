---
name: codex-monitor
description: Use when the user mentions codex-monitor, `$codex-monitor`, cdxm, `$cdxm agmsg`, agmsg joined-team monitor auto-apply, loaded thread injection without resume/fork, app-server control sockets, monitor adapters such as agmsg/HMSG, LaunchAgent watchers, or Codex App remote-control diagnostics.
---

# Codex Monitor

## Operating Rule

Use `cdxm` / `codex-monitor` as the local-first monitor into already loaded Codex App / Codex CLI
threads. Prefer cwd-based auto targeting. Do not use `codex resume` or any
workflow that can fork a thread when the user's intent is to inject into an
existing loaded session.

For Codex CLI threads, make app-server-bound startup the default operating
assumption. Daily CLI sessions should be launched through a `codex` shim that is
first on PATH and routes interactive Codex through app-server/`--remote`; the
existing agmsg `~/.agents/bin/codex` shim is an acceptable implementation of
that contract. A plain real-Codex TUI process that was not launched with
`--remote` is not a reliable live injection target.

Keep the product boundary source-agnostic:

- `target` / thread detection finds a loaded thread by cwd or explicit thread id.
- `sources` adapters poll external event stores and format `BridgeEvent` input.
- `delivery` sends formatted input and advances the adapter cursor only after
  app-server acknowledgement.

Treat `agmsg` as the first adapter example. Prefer `cdxm monitor watch <adapter>`
for new workflows; `cdxm agmsg watch ...` is a source-specific shortcut.

## Chat Shortcuts

When the user sends exactly `$codex-monitor`, treat it as a request to apply the
codex-monitor agmsg receiver for this Codex session's current persona in the
current cwd. Multiple Codex sessions in one cwd are supported; do not collapse
them to a single cwd-level identity.
Do not explain syntax first. Run:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-agmsg-apply.sh [cwd]
```

The helper first resolves explicit arguments/env/session state in this order:
`--team/--name`, `CDXM_MONITOR_TEAM/CDXM_MONITOR_NAME`, `AGMSG_TEAM/AGMSG_AGENT`,
`AGMSG_CODEX_NAME`, then the agmsg `codex-name.<project>.<thread>` marker written
by `/agmsg actas`. It also resolves the current `CODEX_THREAD_ID` or latest
matching Codex session id and passes `--thread` when available, so multiple Codex
threads sharing one cwd do not fight over cwd-only delivery. If no current
persona can be identified and `whoami.sh <cwd> codex` returns `multiple=true`,
ask the user to run `/agmsg actas <name>` or send `$codex-monitor <team> <name>`;
do not pick a persona silently. If the current conversation already established
the active persona for this session, pass it explicitly with `--team/--name`
instead of asking again. If the user typed `$codex-monitor <team> <name>`, pass
`--team <team> --name <name>` to the helper.

Treat explicit `$codex-monitor` as intent to prefer codex-monitor over the
legacy agmsg `codex-bridge` for the same `team/name`. If doctor reports an
active target consumer with `kind=codex-bridge`, the helper should first prove
codex-monitor delivery with dry-run, then replace only that same `team/name`
legacy bridge before foreground watch or LaunchAgent install. Do not stop
unrelated consumers or other roles. `--dry-run-only` remains read-only, and
`--no-replace-legacy` keeps the legacy bridge in place.

An active codex-monitor consumer is only current when the pinned `--thread`
matches the thread selected for this session. If the same `team/name` is already
running with a different pinned thread, treat it as stale: run dry-run, refresh
the LaunchAgent with `--force --load`, and use `launch-agent status` /
`doctor` `desired_thread`, `active_thread`, and `args_match` fields to verify
the running job actually moved.

When the user sends exactly `$cdxm agmsg` or a terse variant like
`$cdxm agmsg <team> <name>`, treat it as an operational request to optimize the
current cwd for codex-monitor-backed agmsg receiving. Do not reply with syntax only.

Default to the current workspace cwd unless the user supplies another cwd. Run
the read-only snapshot first:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-context.sh [cwd] [team] [name]
```

If the command did not include an exact `team/name`, infer it only when runtime
state has exactly one unambiguous identity for the cwd; otherwise ask for the
exact `team/name`. Do not guess between multiple agmsg identities.

Once `team/name` is known, run:

```bash
cdxm agmsg doctor --team <team> --name <name> --cwd <cwd>
cdxm monitor watch agmsg --team <team> --name <name> --cwd <cwd> --dry-run
```

If the user only typed `$cdxm agmsg`, stop after the safe diagnosis/dry-run and
report the exact next command. Start foreground watch or install/load a
LaunchAgent only when the user explicitly asks to start, enable, apply, install,
load, or make the receive bridge durable. Before starting or installing, confirm
that the target thread is loaded/mutable and no competing consumer is already
delivering the same `team/name`.

For a quick read-only snapshot, run:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-context.sh [cwd] [team] [name]
```

Use the optional `team name` arguments only when checking a specific agmsg
LaunchAgent. The script does not start or stop watchers.

## Standard Workflow

1. Confirm `cdxm` is installed and current:

```bash
command -v cdxm
cdxm --help
```

If the repo was just edited, reinstall from the checkout:

```bash
cargo install --path . --bins --force --debug
```

On Windows native PowerShell, install from the checkout with:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

Use `-InstallShim` only when the user explicitly wants the Codex CLI shim:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Yes -InstallShim
```

For the Windows Codex App itself, use the separate reversible shared-server
bridge. This is required when delivery must reach the exact visible App thread:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Yes -NoShim -NoPath -InstallAppBridge -Source .
```

The installer must stage both the App-bundled Codex executable and its matching
`codex-code-mode-host.exe` (plus available command-runner and sandbox helpers)
in `~/.codex-monitor/runtime`. If `-RealCodexPath` is supplied explicitly, its
directory must contain the matching code-mode host.

After Codex App or codex-monitor is updated, fully quit Codex App and run from
any directory:

```powershell
codex-monitor update
```

This updates all three codex-monitor executables from a checksum-verified
Windows release and refreshes the matching private App runtime in one
rollback-safe transaction. It refuses to run while the App bridge/runtime is
active, preserves the owned `CODEX_CLI_PATH` / `CDXM_REAL_CODEX` configuration,
and does not manage watcher lifecycle. Reopen Codex App only after the helper
prints the completion message.

Restart Codex App, then require `cdxm targets` to show
`codex-app-bridge` and `cdxm --target app loaded` to include the visible
thread. An ordinary `codex-app-server-process` endpoint is not a valid App
target. Apply an agmsg receiver for that exact App endpoint with
`cdxm-agmsg-apply.sh --target app ...`. Roll back with
`-SkipBuild -RemoveAppBridge` and another App restart.

For daily Codex CLI monitor use, confirm the Codex entrypoint is shim-backed:

```bash
type -a codex
```

On Windows, use:

```powershell
Get-Command cdxm
Get-Command codex -All
```

The first `codex` should be a shim, normally `$HOME/.agents/bin/codex` on
macOS/Linux or `%USERPROFILE%\.agents\bin\codex.cmd` on Windows. That shim may
be agmsg's shim; the important property is that interactive CLI launches become
app-server-bound so `cdxm targets` can discover the live endpoint without the
user manually typing `--remote`.

When `CODEX_MONITOR_REAL_CODEX` is not set, automatic shim discovery treats the
Windows Desktop-bundled `%LOCALAPPDATA%\OpenAI\Codex\bin\codex` as a final
fallback. A package-manager Codex elsewhere on PATH is preferred so an older
Desktop copy cannot silently downgrade CLI sessions.

Windows builds support the `cdxm` CLI, WebSocket/stdio transports, Codex CLI
shim, and the agmsg SQLite adapter. Use native PowerShell for installation and
keep the Windows `codex.cmd` shim first on PATH when you want CLI sessions to be
app-server-bound for `cdxm agmsg watch`. Native SQLite builds use bundled
`rusqlite`; on Windows this requires the Rust MSVC toolchain plus MSVC Build
Tools.

2. Start agmsg monitor work with the read-only runtime snapshot:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-context.sh /path/to/project <team> <name>
```

This is the first diagnostic surface for target endpoints, matching app thread,
agmsg identity ambiguity, delivery processes, and LaunchAgent status. Run it
before installing or replacing a watcher.

3. Inspect live targets and the cwd thread:

```bash
cdxm targets
cdxm threads --cwd /path/to/project
```

`threads --cwd` is the preferred discovery command because it probes live
endpoints for a loaded thread matching that cwd. Hidden diagnostic command
`cdxm loaded` is endpoint-scoped; if several endpoints exist, use `--target app`,
`--target managed`, or `--endpoint <url>`.

4. Inject text safely:

```bash
cdxm send --cwd /path/to/project --text "message"
```

Default `send` behavior returns after app-server ack. It steers into an active
in-progress turn when visible, otherwise starts a new turn. Do not add `--wait`
when injecting into the same live thread that is controlling the command unless
the user explicitly wants to wait for `turn/completed`.

5. Diagnose and dry-run monitor delivery before starting a watcher:

```bash
cdxm agmsg doctor --team <team> --name <agent> --cwd /path/to/project
cdxm monitor watch agmsg --team <team> --name <agent> --cwd /path/to/project --dry-run
```

`agmsg doctor` is the one-shot runtime truth command: targets, matching loaded
threads, saved state key/id, inbox ids, LaunchAgents/log mtimes, active
consumers, and the ack-vs-visible reminder. `monitor watch agmsg --dry-run`
shows which source events would be delivered to which endpoint/thread/mode
without marking state. Dry-run delivery rows are source-agnostic and include
`source`, `cursor`, `event_id`, and adapter metadata such as `agmsg_id`.

6. Run monitor delivery with the agmsg adapter:

```bash
cdxm monitor watch agmsg --team <team> --name <agent> --cwd /path/to/project
```

This uses the same non-waiting auto send path: active turn -> `turn/steer`,
idle thread -> `turn/start`, then mark the agmsg row seen after ack.
Use `--mode auto|start|steer` when diagnosing active-turn behavior.
`cdxm agmsg watch ...` is a source-specific shortcut for the same adapter.
The watcher retains the logical target and reconnects after endpoint, setup,
or delivery failures. Before sending a pending event, `--target app` and
`--target auto` re-resolve the logical target. If the connected endpoint or
thread drifted, the watcher closes it without sending or saving the cursor,
even when the old app-server still responds, then reconnects and retries the
same event. Explicit and managed targets do not add this discovery probe.
For `--target app --thread <id>`, resolve the current App bridge first, then
verify the pinned thread on the new connection. Retry without calling
`thread/resume` until the App reports it loaded; do not retain the old endpoint
merely because the replacement has not listed the thread yet.
App-server ack and saved state do not prove the current Codex
UI rendered a separate bubble: active-turn `turn/steer` input may reach the
model without one. Verify model receipt in the intended loaded thread; test a
separate visible event only when UI rendering itself is required.

7. Manage durable macOS watch processes:

```bash
cdxm agmsg launch-agent print --team <team> --name <agent> --cwd /path/to/project
cdxm agmsg launch-agent status --team <team> --name <agent>
cdxm agmsg launch-agent install --team <team> --name <agent> --cwd /path/to/project --force --load
cdxm agmsg launch-agent uninstall --team <team> --name <agent>
```

Install or load a LaunchAgent only after the user has selected the exact
`team/name`. If agmsg identity discovery reports multiple agents, do not choose
between them silently. Check existing bridges before installing to avoid double
delivery.

## Safety Checks

- Prefer `cdxm threads --cwd <cwd>` over manual thread ids.
- Pass `--endpoint` only when the user asks for a specific socket or auto is
  ambiguous.
- Use `cdxm targets` before diagnosing endpoint ambiguity.
- Use `cdxm agmsg doctor --team <team> --name <agent> --cwd <cwd>` before
  installing or replacing a watcher.
- Use `cdxm monitor watch agmsg ... --dry-run` before live delivery when the target
  thread or mode is uncertain.
- Use `cdxm --target app remote doctor` for Codex App remote-control device-list
  issues.
- Use `cdxm --target app remote connect --max-messages 1` as the explicit probe
  when working toward phone-like remote-control client behavior. A successful
  connect proves the local enrolled controller/device-key websocket path; it
  does not by itself open or load a thread.
- If `remote doctor` emits `doctor<TAB>device-key-next<TAB>repair-local-controller-enrollment`,
  the local controller enrollment requires Codex App Settings remote-control
  re-authorization before `cdxm` can behave like the phone. Do not silently
  recreate controller device keys.
- Do not stop existing `codex-bridge`, `cdxm agmsg watch`, or LaunchAgent
  processes unless the user explicitly asks. `$codex-monitor` apply is explicit
  replacement intent for the same `team/name` legacy `codex-bridge`, and
  refresh intent for a same `team/name` codex-monitor watcher whose pinned
  thread no longer matches the selected thread.
- If a live delivery test is needed, use a temporary agmsg DB and unique
  `team/name`, then remove the temporary DB after proof is collected.

## Verification Surface

For repo changes, run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

For live smoke without sending arbitrary text:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-context.sh /path/to/project <team> <name>
cdxm targets
cdxm threads --cwd /path/to/project
cdxm agmsg doctor --team <team> --name <agent> --cwd /path/to/project
cdxm monitor watch agmsg --team <team> --name <agent> --cwd /path/to/project --dry-run
cdxm agmsg launch-agent status --team <team> --name <agent>
```

For actual live agmsg smoke, verify both the visible `agmsg monitor event` and
the state entry under `$HOME/Library/Caches/codex-monitor/state.json`.

## More Detail

Read `references/codex-monitor-operations.md` when you need command recipes, LaunchAgent
details, or troubleshooting notes.
