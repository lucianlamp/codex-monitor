# Codex Monitor Operations Reference

## Commands

Read-only discovery:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-context.sh /path/to/project dev kimura
~/.codex/skills/codex-monitor/scripts/cdxm-agmsg-apply.sh /path/to/project --dry-run-only
cdxm targets
cdxm threads --cwd /path/to/project
cdxm agmsg doctor --team dev --name kimura --cwd /path/to/project
cdxm agmsg launch-agent status --team dev --name kimura
```

Joined-persona apply:

```bash
~/.codex/skills/codex-monitor/scripts/cdxm-agmsg-apply.sh /path/to/project
~/.codex/skills/codex-monitor/scripts/cdxm-agmsg-apply.sh /path/to/project --team dev --name kimura
```

The apply helper supports multiple Codex sessions in the same cwd by resolving
the current session persona from explicit args/env, `AGMSG_CODEX_NAME`, or the
agmsg thread-name marker written by `/agmsg actas`. When it can resolve the
current thread, it passes `--thread` through to doctor, dry-run, foreground watch,
and LaunchAgent install. It only asks for a persona when no session-specific
identity exists and `whoami.sh` still reports `multiple=true`.

Codex App native receive shortcuts:

```text
$codex-monitor
$codex-monitor heartbeat
$codex-monitor off
```

The default runs `cdxm-agmsg-foreground.sh <team> <name>` inside the current
turn. Heartbeat mode upserts one current-thread Codex automation. Off cancels
the foreground wait and deletes only that task's matching heartbeat. All three
use agmsg scripts and leave watcher/process lifecycle untouched.

Explicit apply prefers codex-monitor over the legacy agmsg `codex-bridge` for
the same `team/name`: if `doctor` reports `kind=codex-bridge` as the target
consumer, the helper runs dry-run first, then stops only that same legacy bridge
before foreground watch or LaunchAgent install. `--dry-run-only` does not stop
anything; use `--no-replace-legacy` to keep the old bridge even during apply.

Safe live text injection:

```bash
cdxm send --cwd /path/to/project --text "message"
cdxm send --cwd /path/to/project --mode steer --text "message"
```

Pinned delivery when auto is ambiguous:

```bash
cdxm --endpoint unix:///path/to/app-server.sock send --thread <thread-id> --text "message"
```

Codex CLI entrypoint:

```bash
type -a codex
export PATH="$HOME/.agents/bin:$PATH"
```

For normal CLI monitor operation, `codex` should resolve first to a shim that
launches interactive sessions through app-server/`--remote`. The existing agmsg
`~/.agents/bin/codex` shim is acceptable. Without that shim or an explicit
`codex --remote ...`, a plain real-Codex TUI session is usually not injectable by
cwd because there is no loaded app-server endpoint for `cdxm` to attach to.

Foreground monitor watch with the agmsg adapter:

```bash
cdxm monitor watch agmsg --team dev --name kimura --cwd /path/to/project --dry-run
cdxm monitor watch agmsg --team dev --name kimura --cwd /path/to/project
cdxm monitor watch agmsg --team dev --name kimura --cwd /path/to/project --mode start
```

`cdxm agmsg watch ...` is a source-specific shortcut for the same adapter.

LaunchAgent lifecycle:

```bash
cdxm agmsg launch-agent print --team dev --name kimura --cwd /path/to/project
cdxm agmsg launch-agent install --team dev --name kimura --cwd /path/to/project --force
cdxm agmsg launch-agent install --team dev --name kimura --cwd /path/to/project --force --load
cdxm agmsg launch-agent status --team dev --name kimura
cdxm agmsg launch-agent uninstall --team dev --name kimura
```

Remote-control diagnostics:

```bash
cdxm --target app remote doctor
```

Advanced hidden diagnostics/recovery:

```bash
cdxm --target app loaded
cdxm --target app remote status
cdxm --target app remote clients
cdxm --target app remote monitor --count 1
cdxm --target app remote claim --manual-pairing-code <code>
cdxm --target app remote connect
```

## Runtime Facts

- Codex Monitor core is source-agnostic: thread detection resolves loaded Codex threads,
  source adapters poll/format events, and delivery advances cursor state only
  after app-server acknowledgement.
- `--target auto` discovers environment endpoint variables, the Codex App
  control socket, live `codex --remote ...`, `codex app-server --listen ...`,
  and agmsg codex bridge processes.
- `--target app` uses the Codex App control socket on Unix. On Windows it is
  intentionally unavailable because native App has no safe external injection
  endpoint; use the foreground or heartbeat skill shortcut instead.
- cwd-based commands probe `thread/loaded/list` plus `thread/list` to choose a
  live endpoint with a loaded thread for that cwd.
- Explicit `--thread` commands probe `thread/loaded/list` and refuse unloaded
  live targets rather than calling `thread/resume`.
- `send` returns after `turn/start` or `turn/steer` ack by default. `--wait`
  waits for `turn/completed`.
- `monitor watch <adapter>` marks adapter cursors seen only after the app-server
  accepts the delivery. Dry-run rows print `source`, `cursor`, `event_id`, and
  adapter metadata such as `agmsg_id`.
- The agmsg adapter polls only unread inbox rows (`read_at IS NULL`). Already
  read history is ignored even if the codex-monitor cursor state starts empty.
- App-server ack and codex-monitor state advancement are not proof that the current Codex
  UI rendered a visible event. Treat visible `agmsg monitor event` as a separate
  smoke criterion.
- `agmsg doctor` is the runtime truth command before installing or replacing a
  watcher: targets, matching loaded threads, state key/id, inbox ids,
  LaunchAgents/log mtimes, desired vs active LaunchAgent arguments, active
  consumers, pinned consumer thread, and same-team processes.
- LaunchAgent labels are stable per team/name:
  `com.local.codex-monitor.agmsg.<team>.<name>`.

## Choosing Team And Name

Use agmsg helper scripts before starting or installing delivery for a shared
team:

```bash
~/.agents/skills/agmsg/scripts/whoami.sh /path/to/project codex
~/.agents/skills/agmsg/scripts/team.sh dev
~/.agents/skills/agmsg/scripts/delivery.sh status
```

If `whoami.sh` reports `multiple=true`, keep multiple Codex sessions supported:
first use an explicit `--team/--name`, `AGMSG_TEAM/AGMSG_AGENT`,
`AGMSG_CODEX_NAME`, or the `/agmsg actas` thread marker when present. If none
exists, ask which exact role this session should receive as. Do not choose
`goro`, `kimura`, `saburo`, or another role arbitrarily.

## Live Smoke Pattern

Use a temporary DB and unique team/name:

```bash
tmpdir=$(mktemp -d)
db="$tmpdir/messages.db"
sqlite3 "$db" 'CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, team TEXT NOT NULL, from_agent TEXT NOT NULL, to_agent TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL, read_at TEXT);'
sqlite3 "$db" "INSERT INTO messages (team, from_agent, to_agent, body, created_at, read_at) VALUES ('cdxm-smoke', 'cdxm-smoke', 'target', 'smoke', datetime('now'), NULL);"
cdxm monitor watch agmsg --team cdxm-smoke --name target --cwd /path/to/project --agmsg-db "$db"
```

Use `--dry-run` first if the endpoint, thread, or mode is unclear. Stop the
watcher after the state entry is updated, verify the visible `agmsg monitor
event` separately in the intended thread, and remove the temp directory.

## Troubleshooting

- Multiple endpoints: run `cdxm targets`, then retry with `--target app` or
  `--endpoint <url>`.
- No matching cwd thread: open the target project thread in Codex App/CLI, then
  rerun `cdxm threads --cwd <path>`.
- No agmsg delivery: run `cdxm agmsg doctor --team <team> --name <name> --cwd
  <path>` and verify the message row's `team`, `to_agent`, and id are greater
  than the saved state for `agmsg:<team>:<name>`.
- Stale LaunchAgent args: compare `launch-agent status` `desired_thread`,
  `active_thread`, and `args_match`. If the plist was updated but launchd is
  still running old arguments, reinstall with `--force --load`; the installer
  bootouts an existing job before bootstrapping the updated plist.
- Stale LaunchAgent errors: compare `launch-agent status` stderr mtime with the
  latest state id and `doctor` inbox rows before assuming the current watcher is
  failing.
- Device list does not load in Codex settings: run
  `cdxm --target app remote doctor` and inspect app-server clients, backend
  clients, local enrollment, and device-key rows.
