# Windows Codex App bridge integrity A/B design

## Goal

Determine whether Codex Browser Use is blocked by the presence of
`CODEX_CLI_PATH`, by the bridge executable's directory, or by the bridge's
unsigned integrity identity. The experiment must preserve the visible Codex App
thread, avoid process-wide termination, and leave a deterministic rollback to
the native App runtime.

## Current evidence

- With `CODEX_CLI_PATH` unset, the packaged App runtime loaded Google and
  Playwright returned a non-empty DOM snapshot.
- An unsigned bridge in `~/.codex-monitor/bin`, even when it preserved App stdio
  and launched the App-managed signed `codex.exe`, was rejected by Browser Use
  as enterprise network policy.
- The App-managed and WindowsApps `codex.exe` files have the same SHA-256 and a
  valid OpenAI Authenticode signature. The bridge is unsigned.
- The App-managed hashed runtime directory is user-writable, so a separate
  bridge filename can be staged there without replacing an OpenAI file.
- The signed CLI exposes `app-server daemon` and `app-server proxy`, but daemon
  lifecycle returns `only supported on Unix platforms`. The current App bundle
  also guards local-daemon use with `process.platform !== "win32"`.

## Test matrix

Run the variants in this order. Each variant requires a full App restart and
must be accepted against the same visible thread.

### A. Explicit signed native baseline

- Set `CODEX_CLI_PATH` to the App-managed hashed `codex.exe`.
- Clear `CDXM_REAL_CODEX` because no wrapper is involved.
- Require the App child process to be that signed executable with the native
  `app-server` arguments.
- Navigate the in-app Browser to `https://www.google.com/` and require title
  `Google` plus a non-empty Playwright DOM snapshot.

This isolates whether merely defining `CODEX_CLI_PATH` changes Browser policy.
If it fails, the same-folder bridge variant cannot succeed and is skipped.

### B. Unsigned bridge beside the signed runtime

- Copy the bridge to a new, codex-monitor-owned filename in the same hashed
  runtime directory. Never overwrite `codex.exe` or a bundled companion.
- Set `CODEX_CLI_PATH` to that bridge and `CDXM_REAL_CODEX` to the sibling signed
  `codex.exe`.
- Require the bridge and child paths to share the directory, require the child
  to have no `--listen`, and require `cdxm --target app loaded` to include the
  visible thread.
- Repeat the Google title and non-empty DOM acceptance.

This isolates directory placement while keeping the bridge signature unchanged.

## Interpretation

- A passes and B passes: same-directory placement is viable. Update the
  installer and updater to discover the current hashed runtime, stage only the
  owned bridge filename, and remove stale owned copies after the App exits.
- A passes and B fails: the unsigned launcher or another integrity property is
  the blocker. Do not ship path, rename, junction, or self-signing workarounds.
- A fails: any explicit `CODEX_CLI_PATH` changes this App's trust identity. Keep
  the App native and use the App heartbeat receiver.

## Safety and rollback

- Never stop a process by executable name. The user performs each full App
  restart.
- Do not replace or stop existing `cdxm` or agmsg watchers.
- Before each restart, record the intended environment and exact test file
  hashes. After restart, inspect the new bridge/child PIDs but do not terminate
  them.
- Rollback clears the two user environment variables and removes only the
  codex-monitor-owned staged bridge after no process is using it.
- A failed Browser policy response is final for that variant; do not retry via
  raw CDP, another browser, or policy bypass.

## Acceptance

The experiment is complete only when both applicable variants have a recorded
process identity and Browser result, the winning configuration is reflected in
the repository or the native heartbeat fallback is selected, and the final
live App configuration passes Browser, agmsg self-delivery, and CLI version/PATH
checks.

## Recorded results

### Variant A: PASS

- Observed at `2026-07-10T22:39:00+09:00` after a full Codex App restart.
- User `CODEX_CLI_PATH` was the App-managed signed executable
  `C:\Users\ytvar\AppData\Local\OpenAI\Codex\bin\a7c12ebff69fb123\codex.exe`;
  user `CDXM_REAL_CODEX` was empty.
- The App child was PID `70156` (parent PID `49440`) with native arguments
  `-c features.code_mode_host=true app-server --analytics-default-enabled`.
- The executable SHA-256 was
  `B88F944EF63556527CAAE2AD43F80B88B8BE174DC09B09D9B037FC94240A0E91`
  and Authenticode status was `Valid`.
- Browser Use navigated to `https://www.google.com/`, returned title `Google`,
  and produced a non-empty Playwright DOM snapshot of 1,743 characters.

Defining `CODEX_CLI_PATH` is therefore not sufficient to trigger the Browser
policy rejection. Variant B remains applicable.

### Variant B: FAIL

- Observed at `2026-07-10T22:41:38+09:00` after a full Codex App restart.
- User `CODEX_CLI_PATH` was the codex-monitor-owned bridge staged beside the
  native runtime at
  `C:\Users\ytvar\AppData\Local\OpenAI\Codex\bin\a7c12ebff69fb123\cdxm-codex-app-bridge.exe`;
  user `CDXM_REAL_CODEX` was the sibling signed `codex.exe`.
- The bridge was PID `80148` (parent PID `98016`), and its native child was PID
  `88276`. Both executable paths shared the hashed App runtime directory. The
  child retained the native App arguments and had no `--listen` argument.
- The bridge SHA-256 was
  `CCFC541ED9F22F6BD5734DBC3138C4B0B7A0836AED4E0639C49DB89D8955317B`
  with Authenticode status `NotSigned`. The child SHA-256 matched Variant A and
  remained Authenticode `Valid`.
- `cdxm targets` reported exactly one `codex-app-bridge` target, and
  `cdxm --target app loaded` contained the visible thread
  `019f499f-981b-7613-b5cb-8a4d5cdba90b`.
- Browser Use rejected the first navigation to `https://www.google.com/` because
  enterprise network policy blocked it. Per the experiment safety rule, no
  alternate browser surface, URL, raw CDP call, or policy workaround was tried.

Same-directory placement does not restore Browser trust. The winning live
configuration is Variant A: keep `CODEX_CLI_PATH` pointed directly at the signed
App-managed `codex.exe`, keep `CDXM_REAL_CODEX` unset, and use the App heartbeat
receiver for agmsg delivery rather than an unsigned App launcher.

### Native rollback: PASS

- Observed at `2026-07-10T22:53:49+09:00` after the final native App restart.
- The App child was PID `92268` (parent PID `60264`) and used the signed native
  executable with the original App `app-server` arguments.
- User `CODEX_CLI_PATH` remained explicitly pointed at that signed native
  executable, and user `CDXM_REAL_CODEX` was empty.
- No process used the experiment-owned staged bridge, so only
  `a7c12ebff69fb123\cdxm-codex-app-bridge.exe` was removed. No process was
  stopped.
- Browser Use again loaded `https://www.google.com/`, returned title `Google`,
  and produced a Playwright DOM snapshot of 1,743 characters.
