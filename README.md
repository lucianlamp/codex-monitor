# codex-control-bridge

`codex-control-bridge` is a local-first bridge for delivering external events
into the Codex App / Codex app-server control plane.

The short alias binary is `ccb`.

## Commands

```bash
ccb threads --cwd <path>
ccb send --thread <id> --text <msg>
ccb agmsg watch --team <team> --name <agent> --thread <id>
```

## Targets

Default target is `managed`: ccb starts a loopback app-server at
`ws://127.0.0.1:<port>`.

Existing Codex App UI attach is explicit:

```bash
ccb --target app threads --cwd <path>
ccb --target app send --thread <id> --text <msg>
```

On Unix, `--target app` attaches to:

```text
$HOME/.codex/app-server-control/app-server-control.sock
```

`--endpoint ws://127.0.0.1:<port>` connects to an explicit loopback WebSocket.
`--endpoint stdio://` starts an isolated stdio app-server.

## agmsg Adapter

The agmsg adapter reads the message store directly and does not use Codex
shims, PATH replacement, SessionStart hooks, `inbox.sh`, or `watch.sh`.

Default agmsg DB:

```text
$HOME/.agents/skills/agmsg/db/messages.db
```

Override:

```bash
ccb agmsg watch --team dev --name sally --thread <id> --agmsg-db /path/to/messages.db
```

## Safety

- ccb never auto-approves Codex app-server requests.
- ccb refuses non-loopback WebSocket endpoints in the MVP.
- `thread/inject_items` is not the default delivery path.
- `--target app` does not start, stop, or replace the Codex App daemon.
