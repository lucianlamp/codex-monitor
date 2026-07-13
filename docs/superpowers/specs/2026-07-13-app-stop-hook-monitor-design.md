# Codex App Stop Hook Monitor Design

## Goal

Make Codex App `$codex-monitor` wait for agmsg messages at the end of every
turn without heartbeat polling or empty model turns. Keep Codex CLI on its
existing durable background watcher or macOS LaunchAgent so messages can still
reach an in-progress CLI turn through `turn/steer`.

## Scope

This change applies only to the Codex App shortcut family:

- `$codex-monitor` enables the current App session's Stop-hook wait.
- `$codex-monitor off` disables only the current App session's wait.
- `$codex-monitor heartbeat` remains an explicit fallback.

Codex CLI behavior is unchanged. Exact CLI `$codex-monitor` continues to run
`cdxm-agmsg-apply.sh`, creating a Windows background watcher or a macOS
LaunchAgent.

## Architecture

The installed native `codex-monitor` binary owns hook configuration, session
markers, and the Stop-hook protocol. The global hook is dormant unless an exact
session marker exists.

The public App-facing command group provides idempotent operations:

```text
codex-monitor app-hook enable --team <team> --name <name> --session <session-id> --cwd <cwd>
codex-monitor app-hook disable --session <session-id>
codex-monitor app-hook status --session <session-id>
```

`enable` merges one codex-monitor handler into `~/.codex/hooks.json` without
replacing unrelated hooks, then atomically writes a marker under the
codex-monitor user state directory. `disable` removes only that marker.

The configured handler invokes a hidden native command:

```text
codex-monitor __app-stop-hook
```

The executable path in the hook definition is the canonical installed path.
The hook definition includes Unix `command` and Windows `commandWindows`
entries and a 24-hour timeout. Re-running `enable` updates only the exact
codex-monitor handler when its canonical definition has changed and otherwise
leaves `hooks.json` byte-equivalent.

## Session Marker

Each marker is keyed by the Stop-hook `session_id`, which is the current Codex
App thread/session identifier exposed to the shortcut as `CODEX_THREAD_ID`.
The JSON marker contains:

- schema version;
- session id;
- team and agent name;
- canonical cwd;
- creation/update timestamp.

Marker filenames are derived from a validated session id, never from team,
agent, or cwd. Writes use a same-directory temporary file followed by atomic
replacement. A missing, malformed, mismatched, or unsupported marker makes the
hook return success immediately without waiting.

## Stop Hook Data Flow

Codex sends the Stop payload as JSON on standard input. The hidden handler:

1. Parses `session_id`, `cwd`, and `stop_hook_active`, while ignoring unrelated
   payload fields.
2. Loads the exact session marker.
3. Returns `{ "continue": true }` immediately when the marker is absent or the
   cwd does not match.
4. Runs the installed `cdxm-agmsg-foreground.sh <team> <name>` through Bash.
   The helper continues to use the installed agmsg `inbox.sh`, suppressing
   empty polls locally.
5. Converts the first non-empty helper result into the existing visible
   `agmsg monitor event` format.
6. Writes one valid Stop-hook JSON object to standard output:

```json
{
  "decision": "block",
  "reason": "agmsg monitor event\n\nTeam: ..."
}
```

Codex treats the reason as a continuation prompt. When that continuation turn
stops, the marker still exists, so the handler waits again. The handler accepts
`stop_hook_active: true`; re-arming does not spin because each continuation is
blocked on a real inbox message rather than generating an empty prompt.

Hook diagnostics go only to standard error. Standard output always contains
either one valid JSON object or nothing on a fatal nonzero exit.

## Activation and Trust

Codex requires users to review and trust new or changed non-managed hooks.
`enable` never edits Codex's trust state or bypasses that review. When it adds
or changes the handler, it reports that `/hooks` review is required and that a
new App task or App restart may be needed if the current hook registry does not
reload it.

Once trusted, later `$codex-monitor` calls only create or refresh a session
marker and need no repeated review unless the handler definition changes.

## Cancellation

A Stop hook currently waiting owns only its foreground child process. The
handler must propagate termination to that exact child so normal turn
interruption does not leave a detached inbox poller. After interruption,
`$codex-monitor off` removes the current marker, preventing the next Stop event
from waiting.

Neither enable nor disable starts, stops, restarts, or replaces a watcher,
LaunchAgent, heartbeat, or another task's hook marker. Heartbeat cleanup remains
scoped to the current task when the explicit heartbeat mode was used.

## Error Handling

- Existing invalid `~/.codex/hooks.json` blocks installation with a clear
  error; it is never overwritten.
- Missing installed Bash, foreground helper, or agmsg inbox helper makes the
  active hook fail visibly instead of emitting malformed Stop JSON.
- A helper failure is returned as a nonzero hook exit with diagnostics on
  standard error.
- Hook timeout allows Codex to finish the turn; it does not create a synthetic
  message or advance agmsg state.
- Multiple sessions may be enabled concurrently because markers are keyed by
  session id.

## Documentation Changes

The skill and README will define the runtime split before shortcut details:

- Codex App `$codex-monitor` enables Stop-hook waiting.
- Codex App `$codex-monitor off` disables its session marker.
- Codex App heartbeat remains optional fallback behavior.
- Codex CLI `$codex-monitor` remains durable watcher/LaunchAgent apply.

The old App foreground helper remains an internal building block for the Stop
hook, not the user-visible default shortcut.

## Verification

Automated tests cover:

- preserving unrelated global hook entries;
- idempotent handler installation and exact-handler updates;
- rejecting malformed hook configuration without writes;
- atomic marker enable, status, and session-scoped disable;
- marker absence and cwd mismatch as immediate no-ops;
- valid Stop JSON for both `stop_hook_active` values;
- foreground helper failure and message formatting;
- Windows and Unix canonical command generation;
- skill and README App/CLI routing contracts.

Repository verification remains:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Live acceptance uses a temporary HOME and fake agmsg scripts first. Applying
the hook to the real user profile happens only after those tests pass. The real
acceptance then verifies hook discovery, the one-time trust state, session
marker status, one self-addressed visible event, automatic re-arm, and scoped
off behavior without touching any watcher process.
