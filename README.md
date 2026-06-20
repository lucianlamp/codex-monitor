# codex-control-bridge

`codex-control-bridge` is a local-first bridge for delivering external events
into the Codex App / Codex app-server control plane.

The short alias binary is `ccb`.

MVP commands:

```bash
ccb threads --cwd <path>
ccb send --thread <id> --text <msg>
ccb agmsg watch --team <team> --name <agent> --thread <id>
```

Default target is a bridge-managed loopback app-server. Existing Codex App UI
attach is explicit with `--target app` or an explicit `unix://` endpoint.

The core bridge is source-agnostic. agmsg is the first source adapter.
