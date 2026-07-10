# macOS Update and Single-Binary Migration Design

## Context

`codex-monitor` already publishes prebuilt macOS archives for Apple Silicon and
Intel, and `install.sh` can install them. The native `codex-monitor update`
command is still Windows-only, however. Existing macOS installations can also
contain two native binaries (`codex-monitor` and `cdxm`) in more than one
install root, while LaunchAgents continue to execute those old paths.

The live arm64 Mac used for acceptance currently has native binaries in both
`~/.codex-monitor/bin` and `~/.cargo/bin`, and seven
`com.local.codex-monitor.agmsg.*` LaunchAgents whose first program arguments
reference a mixture of those paths.

## Goals

- Make `codex-monitor update` work on macOS arm64 and x86_64.
- Download the matching release tarball and require a valid SHA-256 checksum.
- Preserve the existing transactional binary replacement and rollback model.
- Leave exactly one native executable at
  `~/.codex-monitor/bin/codex-monitor`.
- Keep `cdxm` as a small POSIX launcher that `exec`s the native executable.
- Migrate owned legacy LaunchAgents to the canonical launcher and reload each
  exact label immediately.
- Make `install.sh` use the same macOS migration/finalization behavior.
- Validate the result on the provided Mac over `ssh mac` without sending
  arbitrary monitor messages.

## Non-goals

- Linux self-update support.
- Changes to Codex App endpoint discovery, Browser access, or remote control.
- Automatic creation of new agmsg identities or LaunchAgents.
- Migration of plists outside the
  `com.local.codex-monitor.agmsg.*` namespace.
- Stopping processes by executable name or wildcard.

## Update Architecture

The public command remains `codex-monitor update`. Platform-specific entry
points share checksum parsing, bounded downloads, manifest validation, staged
hash verification, and transactional apply logic.

On macOS, the updater maps the current architecture to one release asset:

- `aarch64` / `arm64` -> `codex-monitor-aarch64-apple-darwin.tar.gz`
- `x86_64` -> `codex-monitor-x86_64-apple-darwin.tar.gz`

The archive and its `.sha256` companion are downloaded into a staging
directory inside `~/.codex-monitor`. The tarball must contain exactly one
top-level regular file named `codex-monitor`; nested paths, links, duplicate
entries, and unexpected files are rejected. The archive and extracted file are
size-bounded. The staged executable receives mode `0755` before it enters the
generic manifest/apply transaction.

macOS can rename an executing binary, so it does not use the Windows detached
helper. The current process stages and verifies the new executable, moves the
old executable into the existing transaction backup, atomically publishes the
new one, verifies its hash, and then removes the verified backup. A replacement
failure restores the previous executable.

The update model becomes platform-aware without changing the single managed
file invariant. `ManagedFile::CodexMonitor` resolves to
`bin/codex-monitor.exe` on Windows and `bin/codex-monitor` on Unix. Release
archive readers supply the same logical file id.

## macOS Installation Finalization

A shared macOS finalizer runs after a successful update and after `install.sh`
places a binary. The installer invokes it through a hidden internal CLI command
so installer and updater migrations cannot drift.

The finalizer performs these steps in order:

1. Atomically write `~/.codex-monitor/bin/cdxm` as a POSIX launcher that resolves
   its own directory and `exec`s `codex-monitor` with all arguments.
2. Inventory plist files whose labels match
   `com.local.codex-monitor.agmsg.*`.
3. Select only plists whose first `ProgramArguments` entry is one of the fixed
   owned paths:
   - `~/.codex-monitor/bin/cdxm`
   - `~/.codex-monitor/bin/codex-monitor`
   - `~/.cargo/bin/cdxm`
   - `~/.cargo/bin/codex-monitor`
4. Snapshot each selected plist byte-for-byte and record whether its exact
   launchd service is loaded.
5. Replace only the first program argument with
   `~/.codex-monitor/bin/cdxm`, publish the plist atomically, and reload that
   exact label when it was previously loaded.
6. Verify that every reloaded service reports the canonical active first
   argument.
7. Only after all selected plists succeed, remove the fixed legacy native files
   under `~/.cargo/bin` and ensure the installed `cdxm` is a launcher rather
   than a native binary.

No process-name enumeration or wildcard termination is used. Immediate
activation is limited to the exact labels discovered from the owned plist
namespace.

## LaunchAgent Rollback

LaunchAgent migration is a second transaction after the binary transaction.
If writing, reloading, or verification fails for any selected label:

- restore every changed plist from its in-memory snapshot;
- reload only services that were loaded before migration;
- keep all legacy executables in place;
- return a failure that names the affected label and any rollback failure.

The newly installed canonical binary remains installed because it is compatible
with both old and new plist arguments. This avoids rolling back a verified
security update solely because launchd rejected one service reload.

## Installer Behavior

`install.sh` keeps its existing prebuilt-first and source-build fallback flow.
After installing `codex-monitor` on Darwin, it calls the installed binary's
hidden macOS finalizer instead of maintaining separate plist migration logic in
shell. `--skip-build` remains side-effect-light for contract tests and does not
reload LaunchAgents.

Fresh installs create the canonical launcher. Upgrades through either
`install.sh` or `codex-monitor update` converge on the same single-binary layout
and LaunchAgent arguments.

## Error Handling and Safety

- Missing checksums fail closed; no unverified binary is installed.
- A checksum mismatch is a hard failure, not a source-build fallback.
- Unsupported architectures fail before writing installation files.
- Downloads and extracted files have explicit size limits.
- Tar paths and entry types are validated before extraction.
- Launcher and plist writes use temporary siblings plus rename.
- Only fixed install paths and exact owned LaunchAgent labels are mutated.
- Existing agmsg databases, cursors, logs, working directories, thread pins,
  modes, and remaining program arguments are preserved.

## Testing

Automated tests cover:

- macOS architecture-to-asset selection;
- strict checksum parsing and mismatch rejection;
- valid and invalid tar.gz archives, including nested files and links;
- platform-aware managed destinations;
- atomic POSIX launcher generation;
- pure plist first-argument migration while preserving all other fields;
- owned-path filtering;
- multi-agent rollback behavior through injected launchctl hooks;
- installer, release workflow, CLI, README, and skill contracts;
- the existing full Rust test suite, formatting, and clippy.

Live `ssh mac` acceptance will:

1. build the branch natively on the arm64 Mac;
2. exercise `codex-monitor update` against a task-owned local release server;
3. install the verified binary and run the macOS finalizer;
4. immediately reload the seven existing exact LaunchAgent labels;
5. verify one native binary, a text `cdxm` launcher, canonical plist and active
   launchd arguments, and loaded service status;
6. stop only the temporary release server started by this task and remove its
   temporary files.

