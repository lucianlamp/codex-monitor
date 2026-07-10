# Native Codex App and Heartbeat Delivery Design

## Problem

The Windows Codex App loses Browser Use trust whenever it starts through the
unsigned `cdxm-codex-app-bridge.exe`. A controlled A/B test proved that moving
the same bridge beside the signed App runtime does not help, while an explicit
`CODEX_CLI_PATH` that points directly to the signed App-managed `codex.exe`
loads Google normally.

The installed product currently ships, enables, and updates the failing bridge
and copies an App runtime solely to support it. That adds a second App launch
path, makes `codex-monitor update` depend on Codex App state, and can terminate
the App/CLI experience through transport failures. agmsg delivery to the App is
already available through a Codex-owned heartbeat task that invokes the agmsg
inbox script without replacing the App executable.

## Goals

- Keep Codex App on a signed native executable, including when
  `CODEX_CLI_PATH` is explicitly set to that executable.
- Keep the npm-backed Codex CLI shim and its monitor path independent from the
  App.
- Make `codex-monitor update` update only the public codex-monitor binaries and
  PATH integration.
- Migrate an existing codex-monitor-owned App bridge without overwriting a
  user-owned or native `CODEX_CLI_PATH`.
- Provide explicit foreground, heartbeat, and off modes through the
  `$codex-monitor` App shortcut.
- Remove bridge-only code, artifacts, runtime copies, options, and docs.

## Non-goals

- Do not proxy, inject into, or remotely control the native Windows App
  app-server.
- Do not sign, rename, junction, or relocate an unsigned launcher to imitate a
  trusted OpenAI executable.
- Do not create, stop, restart, or replace heartbeat tasks or watchers during an
  install or update. Only an explicit `$codex-monitor heartbeat` or
  `$codex-monitor off` request may change the matching App automation.
- Do not change the npm Codex CLI selected by the public shim.
- Do not stop any process by name or wildcard.

## Selected architecture

### Codex App

Codex App starts the OpenAI-signed native `codex.exe` directly. codex-monitor
does not install an App launcher or a private copy of the App runtime.

The updater and installer preserve `CODEX_CLI_PATH` and `CDXM_REAL_CODEX` when
the values are not owned by codex-monitor. This includes the validated state in
which `CODEX_CLI_PATH` explicitly names the signed App-managed executable and
`CDXM_REAL_CODEX` is empty.

### App message delivery

The App shortcut resolves `team/name` with the existing codex-monitor persona
rules. An established persona is reused. Ambiguous identities require the user
to choose; the shortcut never guesses.

#### `$codex-monitor`: foreground wait

The default App action keeps the current assistant turn active in a blocking
tool call. A codex-monitor-owned helper repeatedly invokes only:

```bash
~/.agents/skills/agmsg/scripts/inbox.sh <team> <agent>
```

`No new messages.` is suppressed inside the local helper, so the model is not
resampled for empty polls. When a message arrives, the helper returns it, the
assistant presents it in the current task using the `agmsg monitor event`
format, and immediately enters the blocking wait again. A message that requires
a reply still uses `send.sh`.

The helper is a foreground child of the active tool call. It does not daemonize,
write a PID file, start a watcher, or connect to the App app-server. User steer
can interrupt the wait. After handling an ordinary steer, the assistant resumes
the wait; `$codex-monitor off` ends it.

This mode has no periodic model turn while idle. It ends if Codex App exits, the
task transport disconnects, or the active turn is cancelled.

#### `$codex-monitor heartbeat`: persistent wait

The heartbeat action uses the Codex App automation API to create or update one
active one-minute heartbeat attached to the current task and persona. Its
deterministic name includes the team, agent, and current thread identifier.
Re-running the command updates the matching automation rather than creating a
duplicate.

The heartbeat prompt invokes only `inbox.sh` from the installed agmsg skill. It
stays silent for `No new messages.`, presents returned messages in the current
task, uses `send.sh` for replies, and explicitly forbids starting, stopping,
killing, restarting, replacing, or installing any watcher or process.

Heartbeat mode survives a completed assistant turn and App task inactivity, but
each scheduled check starts a model turn. It is therefore opt-in rather than the
default.

#### `$codex-monitor off`: stop delivery

The off action ends the current foreground wait through normal turn
cancellation and deletes only the heartbeat whose team, agent, and target thread
match the current task. A missing matching heartbeat is a successful no-op. It
does not stop a process by PID or name and does not alter CLI monitor consumers.

codex-monitor documents these boundaries but does not read the agmsg database
directly. The helper and heartbeat use the installed agmsg scripts as their only
message interface.

Both modes keep Browser Use and the App transport native and avoid coupling App
lifetime to an unsigned monitor executable.

### Codex CLI

The existing public shim remains the CLI integration. It resolves the intended
npm Codex installation and uses the CLI-specific monitor endpoint. Its PATH and
processes remain independent from Codex App, so an App restart cannot terminate
an interactive CLI through a shared bridge.

### Release and installer

Windows release archives contain only `codex-monitor.exe` and `cdxm.exe`.
`cdxm-codex-app-bridge.exe` and copied App runtime companions are removed from
the managed file model and release workflow.

The PowerShell installer no longer exposes `-InstallAppBridge`,
`-RemoveAppBridge`, or `-RealCodexPath`. It installs the two public binaries and
the existing CLI shim/PATH integration. It never changes an unowned
`CODEX_CLI_PATH`.

### Legacy migration

A legacy bridge is owned only when both conditions hold:

1. user `CODEX_CLI_PATH` equals
   `<install-root>\bin\cdxm-codex-app-bridge.exe`; and
2. `<install-root>\app-bridge-env.json` is version 1 and records the same
   `bridgePath`.

When those conditions hold, installation or update restores the recorded
`previousCodexCliPath` and `previousCdxmRealCodex` and removes the ownership
file. If the legacy bridge process is still active, migration stops with an
instruction to quit Codex App; it never terminates the process.

The bridge and copied runtime locations under the codex-monitor install root
are fixed obsolete managed paths. They are removed when their exact paths are
not in use, even when the current environment already points to native Codex.
When the environment is already native or otherwise unowned, active orphaned
legacy runtime files do not block installation of the public binaries: their
cleanup is deferred until a later update. This file cleanup does not authorize
any environment change or process termination.

Public binary replacement is also per-file. If an exact installed public
binary path is active, installation or update preserves that file and updates
the other inactive public binary. A later run replaces the deferred file after
its consumer exits; no public process is stopped by the product.

If ownership cannot be proven, environment values are preserved and the
operation reports an actionable warning instead of guessing. A native explicit
`CODEX_CLI_PATH` is therefore stable across future updates.

### `codex-monitor update`

The updater downloads and verifies only the two release binaries, stages a
self-update helper, waits for its own parent PID, applies fixed-destination
files, normalizes the public CLI PATH, and removes known obsolete bridge/runtime
files when safe.

A native App does not block the update. Only an active, proven-owned legacy
bridge blocks its one-time migration. The updater does not copy files from the
Codex App package and does not reassert `CDXM_REAL_CODEX`.

### Windows App target behavior

Without the bridge, `--target app` cannot safely address the native Windows App.
The command returns a concise error directing App delivery to the heartbeat
receiver. CLI target discovery and explicit endpoint behavior remain unchanged.

## Failure handling and safety

- Environment migration requires the ownership file and exact normalized path
  match.
- An invalid or foreign ownership file never authorizes environment changes or
  deletion.
- Cleanup addresses only fixed codex-monitor-owned paths.
- Running processes are inspected only to decide whether migration can proceed;
  no process is stopped.
- Update manifests accept only the two fixed release filenames and reject path
  traversal, duplicates, missing required files, and invalid hashes.
- Failed updates preserve the installed binaries and report their result through
  the existing atomic update-result file.

## Verification

Automated coverage must prove:

- the foreground helper suppresses empty polls, returns the first non-empty
  inbox result, and does not daemonize or manage a watcher;
- the skill maps the three exact App shortcuts to foreground wait, one-minute
  heartbeat upsert, and scoped off behavior;
- heartbeat instructions use only agmsg scripts and reject watcher lifecycle
  changes;
- release manifests and archives contain exactly the two public binaries;
- installer options and docs contain no App bridge enable path;
- an explicit native `CODEX_CLI_PATH` is preserved;
- a proven-owned legacy bridge restores its saved environment;
- an unowned or malformed bridge state is preserved and reported;
- active legacy bridge migration fails without stopping a process;
- obsolete bridge/runtime files are removed only from fixed owned paths;
- `codex-monitor update` no longer requires App package runtime staging;
- Windows `--target app` explains the heartbeat receiver;
- the npm CLI shim resolution tests continue to pass.

Live Windows acceptance requires one installed configuration to pass all of the
following:

1. Codex App child path is the signed native executable and has native
   `app-server` arguments.
2. Browser Use loads `https://www.google.com/` and returns a non-empty DOM.
3. A self-addressed agmsg event reaches the visible task through foreground
   wait without an empty-poll model turn.
4. Heartbeat mode delivers one event after the foreground turn is stopped, and
   `off` removes the matching automation without touching a watcher.
5. `codex --version` and `cdxm --version` resolve through the intended public
   PATH, and a separately launched Codex CLI remains independent from App
   restart.
6. The installed tree contains no App bridge or copied App runtime.

## Rollout

Ship the removal as the next Windows release and install it locally from the
feature branch for acceptance. Preserve the current explicit signed native
`CODEX_CLI_PATH`. Convert the current heartbeat only through the explicit
shortcut acceptance test. If any automated or live check fails, retain the
signed native App configuration and do not re-enable the bridge.
