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

## Shortcut Runtime Routing

Choose the runtime branch before interpreting `$codex-monitor`. Treat the host
as Codex App only when the runtime explicitly identifies itself as Codex App in
the current system context. Otherwise treat the host as Codex CLI. Windows,
the presence of this skill, or a discoverable app-server endpoint do not by
themselves identify Codex App.

In Codex CLI, exact `$codex-monitor` means apply the session-scoped durable
receiver described under **CLI Chat Shortcuts**. Never run
`cdxm-agmsg-foreground.sh` for the CLI shortcut and do not pass `--foreground`
to `cdxm-agmsg-apply.sh`. The apply helper may return after setup, but the
Windows background watcher or macOS LaunchAgent must remain as the durable
receiver. The foreground inbox helper is Codex App-only.

## Codex App Shortcuts

These exact shortcuts apply when the current host is Codex App. They keep the
App on its signed native runtime and use only the installed agmsg scripts as the
message interface. Never route these shortcuts through `cdxm --target app`, an
external App launcher, or a background watcher.

Resolve the current persona in this order:
`--team/--name`, `CDXM_MONITOR_TEAM/CDXM_MONITOR_NAME`, `AGMSG_TEAM/AGMSG_AGENT`,
`AGMSG_CODEX_NAME`, then the agmsg `codex-name.<project>.<thread>` marker written
by `/agmsg actas`. Reuse a persona already established in the conversation. If
no current
persona can be identified and `whoami.sh <cwd> codex` returns `multiple=true`,
ask the user to run `/agmsg actas <name>` or send `$codex-monitor <team> <name>`;
do not pick a persona silently. If the current conversation already established
the active persona, do not ask again.

### `$codex-monitor`: session Stop hook wait

Enable the dormant global Stop hook for only the current App task. Resolve the
current target thread id and cwd, then run:

```bash
codex-monitor app-hook enable --team <team> --name <name> --session <thread-id> --cwd <cwd>
codex-monitor app-hook status --session <thread-id>
```

The installed hook invokes the internal `cdxm-agmsg-foreground.sh` helper only
after a turn stops. Empty inbox polls stay inside the hook process and do not
create model turns. When a message arrives, the hook returns a Stop continuation
whose reason contains each message in this format:

```text
agmsg monitor event

Team: <team>
Recipient: <name>
Sender: <sender>

<message>
```

If a message requires a reply, use the installed agmsg `send.sh`. The marker
remains active, so the next completed turn enters Stop-hook waiting again.
`stop_hook_active=true` is expected during these continuations and must not
disable re-arming.

When `enable` prints `trust-required`, tell the user to open **Codex App
Settings > Hooks**, review the handler whose status is
`Waiting for agmsg via codex-monitor`, and choose **Trust**. The `/hooks`
command is not required.
Never modify `[hooks.state]` or bypass hook trust. A new App task or App restart
may be needed when the current hook registry does not reload a newly added
definition. Do not start, install, replace, or daemonize a watcher.

### `$codex-monitor heartbeat`: one-minute heartbeat

Use tool discovery to find `automation_update`. Resolve the current Codex
target thread id. Inspect existing Codex automations read-only and find the
deterministic current-task name
`agmsg-<team>-<name>-<thread-id>-codex-monitor`. Update the match instead of
creating a duplicate; otherwise create one active one-minute heartbeat with
`targetThreadId` set to the current thread.

The heartbeat prompt must say:

```text
Use the installed agmsg skill and only its scripts. In <cwd>, run via Git Bash:
~/.agents/skills/agmsg/scripts/inbox.sh <team> <name>
If it says "No new messages.", finish silently with no user-facing update. If
it returns messages, present each one in this current Codex task using the
agmsg monitor event format. If a message requires a reply, use
~/.agents/skills/agmsg/scripts/send.sh rather than answering only in chat.
Never start, stop, kill, restart, replace, or install any watcher or process.
```

Use `automation_update`; never edit automation TOML directly and never show a
raw recurrence rule to the user.

### `$codex-monitor off`: current-task cleanup

If the Stop hook is currently waiting, treat the user's interrupt as normal
cancellation of that owned hook call. Resolve the current target thread id and
run:

```bash
codex-monitor app-hook disable --session <thread-id>
```

Resolve the same team/name and target thread, find the matching deterministic
heartbeat, and delete only that automation with `automation_update`. A missing
marker or heartbeat is a successful no-op. Do not stop a PID, watcher, CLI
consumer, or any other task's heartbeat.

## CLI Chat Shortcuts

In Codex CLI, when the user sends exactly `$codex-monitor`, apply the
codex-monitor agmsg durable receiver for this Codex session's current persona
in the current cwd. Multiple Codex sessions in one cwd are supported; do not
collapse them to a single cwd-level identity. Do not substitute the App-only
foreground inbox helper. Run:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-agmsg-apply.sh [cwd]
```

The helper resolves the current thread and persona and passes `--thread` when
available. Treat explicit CLI apply as intent to replace only the same
`team/name` legacy bridge after a successful dry-run. Do not stop unrelated
consumers or other roles. `--dry-run-only` remains read-only, and
`--no-replace-legacy` keeps the legacy bridge in place.

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
cargo install --path . --bin codex-monitor --force --debug
```

On Windows native PowerShell, install from the checkout with:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

Use `-InstallShim` only when the user explicitly wants the Codex CLI shim:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Yes -InstallShim
```

For Windows Codex App, keep `CODEX_CLI_PATH` unset or pointed directly at the
OpenAI-signed native App-managed `codex.exe`. Never point it at a codex-monitor
launcher. Use the Codex App shortcuts above for agmsg delivery.

Run the updater from any directory on Windows or macOS:

```powershell
codex-monitor update
```

On macOS arm64 and Intel, this downloads the matching checksum-verified tar.gz,
atomically updates the one native executable at
`~/.codex-monitor/bin/codex-monitor`, regenerates the POSIX `cdxm` launcher,
migrates owned `com.local.codex-monitor.agmsg.*` plists, and reloads each exact
LaunchAgent that was already loaded. It restores changed plists if any reload
or active-argument verification fails, and removes fixed legacy native copies
under `~/.cargo/bin` only after the whole migration succeeds. Linux self-update
is not supported; use `install.sh` there.

On Windows, this updates the single native `codex-monitor.exe` from a
checksum-verified release, refreshes the `cdxm.cmd` compatibility launcher, preserves an
unowned or explicitly native `CODEX_CLI_PATH`, and
migrates a proven-owned legacy bridge back to its saved environment. A native
App does not need to be closed. When `CODEX_CLI_PATH` is already native or
otherwise unowned, the updater installs the public binaries and defers cleanup
of active orphaned legacy runtime files until a later update. It removes only
inactive fixed legacy paths. If an owned legacy bridge is still active, the
updater asks the user to quit App because restoring that owned environment is
the migration itself. An active legacy `cdxm.exe` is left running and its fixed
file is removed by a later update after the consumer exits. New `cdxm` commands
already resolve through `cdxm.cmd` to `codex-monitor.exe`. It never stops a
process. The updater does not manage
Stop-hook markers, heartbeat automations, watchers, or CLI consumers.

On Windows, `--target app` is intentionally unavailable because native App does
not expose a safe external injection endpoint. Use `$codex-monitor` Stop hook
wait or `$codex-monitor heartbeat`. Browser acceptance remains a native App
check: Google must load with a non-empty Playwright DOM snapshot.

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

When `CODEX_MONITOR_REAL_CODEX` is not set, automatic shim discovery rejects the
Windows Desktop-bundled `%LOCALAPPDATA%\OpenAI\Codex\bin\codex`. If no npm or
other package-manager CLI is available, it fails with
`refusing Windows Desktop Codex fallback` instead of silently downgrading the
terminal session.

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
