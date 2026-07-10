# Codex CLI Runtime Isolation Design

## Problem

Windows currently exposes three Codex layers:

- the public `codex` shim under `~/.agents/bin`;
- the package-manager CLI under `%APPDATA%/npm`;
- a Desktop-installed CLI under `%LOCALAPPDATA%/OpenAI/Codex/bin`.

The Desktop-installed CLI can lag behind the npm CLI. If shim discovery is
bypassed or falls back to the Desktop directory, a new terminal can silently
start an older Codex version. The Codex App also needs its own matching private
runtime and code-mode host, so forcing the App and terminal to share one
physical executable would reintroduce compatibility risk.

## Goals

- Present one public `codex` entrypoint to terminal users.
- Use the npm-installed Codex CLI for ordinary terminal sessions.
- Never silently fall back from npm to the Desktop-installed CLI.
- Keep the App-compatible runtime private to the Codex App bridge.
- Normalize the user PATH during install and `codex-monitor update` so the
  Desktop CLI directory cannot precede or bypass the public shim.
- Detect missing or invalid npm CLI installations with a clear error.

## Non-goals

- Do not delete or modify files owned by the Microsoft Store Codex package.
- Do not make the terminal CLI and Codex App use one physical binary.
- Do not bypass Browser Use enterprise network policy. Browser policy is
  independent of local CLI selection and must be diagnosed or changed through
  its owning policy surface.

## Approaches Considered

### One public entrypoint with isolated runtimes (selected)

The public shim resolves an explicit user override first and npm/package-manager
Codex second. It rejects the Desktop-installed directory and fails closed when
no supported CLI exists. The App bridge continues to use its private staged
runtime through `CDXM_REAL_CODEX`.

This keeps terminal behavior current while preserving the App's exact runtime
and code-mode-host pairing.

### Use the npm CLI for the App and terminal

This would reduce the executable count, but the App can bundle an alpha or
otherwise App-specific runtime and matching helper binaries. Replacing it with
the npm CLI can break code-mode and plugin capabilities.

### Use the App CLI for the App and terminal

This also reduces the executable count, but App updates and the per-user
Desktop CLI alias can lag behind npm. It recreates the silent downgrade that
this change is intended to prevent.

## Design

### Public CLI resolution

`~/.agents/bin/codex.cmd` remains the only supported public entrypoint. The
shared `codex-shim.sh` resolves candidates in this order:

1. `CODEX_MONITOR_REAL_CODEX`, when explicitly configured and executable.
2. A non-shim package-manager candidate, including the npm installation.
3. A clear failure explaining how to install `@openai/codex` or configure the
   explicit override.

Candidates under `%LOCALAPPDATA%/OpenAI/Codex/bin` are never used by the public
shim, even when they are the only remaining `codex` on PATH.

### PATH normalization

The Windows installer and `codex-monitor update` maintain an idempotent user
PATH with these properties:

- `~/.agents/bin` is present before package-manager paths;
- `%APPDATA%/npm` remains available for the real CLI;
- `%LOCALAPPDATA%/OpenAI/Codex/bin` is removed from the user PATH;
- unrelated PATH entries and their relative order are preserved.

Before the first codex-monitor-owned PATH normalization, the original user PATH
is written once to `~/.codex-monitor/user-path-backup.json`. Later installs and
updates preserve that original backup instead of replacing it with an already
normalized value.

The App package may recreate its own directory or executable, but it will not
be selected through the user PATH after the next install or update repair.

### App runtime isolation

The App bridge continues to stage the App-compatible Codex executable and
matching helper binaries under `~/.codex-monitor/runtime`. Only the bridge uses
`CDXM_REAL_CODEX`; the private runtime is not placed on PATH and is not exposed
as the terminal `codex` command.

### Diagnostics and errors

Installer/update output reports the selected public shim, npm CLI, and private
App runtime separately. A missing npm CLI is actionable and does not trigger a
Desktop fallback. Explicit overrides remain supported for advanced users and
are reported as such.

## Browser Capability Boundary

Using the current npm CLI removes an old-runtime compatibility blocker for CLI
plugins. It does not override Browser Use policy. The current in-app Browser
connection initializes successfully, but navigation to `https://www.google.com`
is rejected by enterprise network policy. Browser policy remediation is a
separate follow-up and must not be implemented as a workaround in the shim or
bridge.

## Verification

Automated tests cover:

- Desktop before npm on PATH still resolves npm;
- Desktop as the only candidate fails with the expected message;
- an explicit non-shim override still wins;
- installer and update PATH normalization is idempotent and preserves unrelated
  entries;
- App runtime staging and bridge resolution remain unchanged.

Live verification covers:

- `Get-Command codex -All` shows the public shim first;
- a newly launched PowerShell `codex` process runs the npm executable;
- `codex --version` matches the npm package version;
- the Codex App bridge runs the private staged runtime;
- the existing CLI and App monitor connections remain alive during installation
  checks that do not require an App restart.

## Rollback

Reinstalling the previous codex-monitor release restores its shim and updater.
The original user PATH can be restored from
`~/.codex-monitor/user-path-backup.json` without touching Microsoft Store
package files.
