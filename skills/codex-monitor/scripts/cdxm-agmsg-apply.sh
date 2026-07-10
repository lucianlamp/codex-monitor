#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: cdxm-agmsg-apply.sh [cwd] [--team <team> --name <agent>] [--mode auto|start|steer] [--target auto|app|managed] [--dry-run-only] [--foreground] [--no-replace-legacy]

Infer the current agmsg codex identity for cwd, run codex-monitor diagnostics,
and apply a durable LaunchAgent monitor for the current session persona.
EOF
}

project=""
team=""
name=""
mode="${CDXM_MONITOR_MODE:-auto}"
target="${CDXM_MONITOR_TARGET:-auto}"
apply=1
foreground=0
replace_legacy=1

while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --team)
      team="${2:?--team requires a value}"
      shift 2
      ;;
    --name)
      name="${2:?--name requires a value}"
      shift 2
      ;;
    --mode)
      mode="${2:?--mode requires a value}"
      shift 2
      ;;
    --target)
      target="${2:?--target requires a value}"
      shift 2
      ;;
    --dry-run-only|--no-apply)
      apply=0
      shift
      ;;
    --foreground)
      foreground=1
      apply=0
      shift
      ;;
    --replace-legacy)
      replace_legacy=1
      shift
      ;;
    --no-replace-legacy)
      replace_legacy=0
      shift
      ;;
    --*)
      printf 'unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [ -n "$project" ]; then
        printf 'unexpected extra argument: %s\n' "$1" >&2
        usage >&2
        exit 2
      fi
      project="$1"
      shift
      ;;
  esac
done

project="${project:-$(pwd)}"
project="$(cd "$project" && pwd)"

case "$mode" in
  auto|start|steer) ;;
  *)
    printf 'invalid --mode: %s\n' "$mode" >&2
    exit 2
    ;;
esac

case "$target" in
  auto|app|managed) ;;
  *)
    printf 'invalid --target: %s\n' "$target" >&2
    exit 2
    ;;
esac

codex_monitor_bin="${CODEX_MONITOR_BIN:-}"
if [ -z "$codex_monitor_bin" ]; then
  if command -v cdxm >/dev/null 2>&1; then
    codex_monitor_bin="$(command -v cdxm)"
  elif command -v codex-monitor >/dev/null 2>&1; then
    codex_monitor_bin="$(command -v codex-monitor)"
  fi
fi

if [ -z "$codex_monitor_bin" ]; then
  printf 'cdxm/codex-monitor not found on PATH\n' >&2
  exit 127
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_dir="$(cd "$script_dir/.." && pwd)"
agmsg_dir="${AGMSG_SCRIPTS_DIR:-$HOME/.agents/skills/agmsg/scripts}"
whoami_script="$agmsg_dir/whoami.sh"
identities_script="$agmsg_dir/identities.sh"
delivery_script="$agmsg_dir/delivery.sh"
context_script="$script_dir/cdxm-context.sh"
agmsg_run_dir="${AGMSG_RUN_DIR:-$HOME/.agents/skills/agmsg/run}"

if [ ! -x "$whoami_script" ]; then
  printf 'agmsg whoami helper not found: %s\n' "$whoami_script" >&2
  exit 127
fi
if [ ! -x "$identities_script" ]; then
  printf 'agmsg identities helper not found: %s\n' "$identities_script" >&2
  exit 127
fi

target_args=(--target "$target")
if [ ! -x "$delivery_script" ]; then
  printf 'agmsg delivery helper not found: %s\n' "$delivery_script" >&2
  exit 127
fi

section() {
  printf '\n[%s]\n' "$1"
}

extract_field() {
  local text="$1"
  local key="$2"
  printf '%s\n' "$text" | tr ' ' '\n' | awk -F= -v key="$key" '$1 == key { print substr($0, index($0, "=") + 1); exit }'
}

extract_tab_field() {
  local text="$1"
  local key="$2"
  printf '%s\n' "$text" | tr '\t' '\n' | awk -F= -v key="$key" '$1 == key { print substr($0, index($0, "=") + 1); exit }'
}

stop_legacy_codex_bridge_consumers() {
  local consumers="$1"
  local line pid args pidfile stopped=0
  stop_pid() {
    local pid="$1"
    case "$(uname -s 2>/dev/null || printf 'unknown')" in
      MINGW*|MSYS*|CYGWIN*)
        MSYS2_ARG_CONV_EXCL='*' taskkill.exe /PID "$pid" /T /F >/dev/null 2>&1 \
          || powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass \
            -Command "Stop-Process -Id $pid -Force -ErrorAction Stop" >/dev/null 2>&1
        ;;
      *)
        kill "$pid" 2>/dev/null
        ;;
    esac
  }
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    case "$line" in
      *$'\tkind=codex-bridge'*) ;;
      *) continue ;;
    esac
    pid="$(extract_tab_field "$line" pid)"
    [ -n "$pid" ] || continue
    args="$(extract_tab_field "$line" command)"
    case "$args" in
      *codex-bridge.js*|*codex-bridge-launcher.sh*) ;;
      *) continue ;;
    esac
    if stop_pid "$pid"; then
      stopped=$((stopped + 1))
    fi
  done <<<"$consumers"

  pidfile="$agmsg_run_dir/codex-bridge.$team.$name.pid"
  if [ -f "$pidfile" ]; then
    pid="$(cat "$pidfile" 2>/dev/null || true)"
    stop_pid "$pid" >/dev/null 2>&1 || true
    rm -f "$pidfile" "${pidfile%.pid}.meta"
  fi
  printf '%s' "$stopped"
}

stop_windows_legacy_codex_bridge_consumers() {
  case "$(uname -s 2>/dev/null || printf 'unknown')" in
    MINGW*|MSYS*|CYGWIN*) ;;
    *) printf '0'; return 0 ;;
  esac

  TEAM="$team" NAME="$name" powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command '
$team = $env:TEAM
$name = $env:NAME
$killed = 0
Get-CimInstance Win32_Process |
  Where-Object {
    $_.CommandLine -like "*codex-bridge.js*" -and
    $_.CommandLine -like "*--team $team*" -and
    $_.CommandLine -like "*--name $name*"
  } |
  ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
    $killed += 1
  }
[Console]::Write($killed)
' 2>/dev/null || printf '0'
}

resolve_thread() {
  if [ -n "${CODEX_THREAD_ID:-}" ]; then
    printf '%s' "$CODEX_THREAD_ID"
    return 0
  fi
  local sessions_dir="$HOME/.codex/sessions" f first esc cwd tid
  [ -d "$sessions_dir" ] || return 0
  while IFS= read -r f; do
    [ -f "$f" ] || continue
    first="$(head -1 "$f" 2>/dev/null || true)"
    case "$first" in *'"session_meta"'*) ;; *) continue ;; esac
    esc="$(printf '%s' "$first" | sed "s/'/''/g")"
    cwd="$(sqlite3 ":memory:" "SELECT COALESCE(json_extract('$esc','\$.payload.cwd'),'')" 2>/dev/null || true)"
    [ "$cwd" = "$project" ] || continue
    tid="$(sqlite3 ":memory:" "SELECT COALESCE(json_extract('$esc','\$.payload.id'),'')" 2>/dev/null || true)"
    [ -n "$tid" ] && { printf '%s' "$tid"; return 0; }
  done <<EOF
$(ls -t "$sessions_dir"/*/*/*/rollout-*.jsonl 2>/dev/null | head -20)
EOF
  return 0
}

resolve_team_for_name() {
  local wanted_name="$1"
  "$identities_script" "$project" codex \
    | awk -v n="$wanted_name" 'NF >= 2 && $2 == n { print $1 }' \
    | awk '!seen[$0]++'
}

codex_entrypoint_kind() {
  local codex_first
  codex_first="$(command -v codex 2>/dev/null || true)"
  if [ -z "$codex_first" ] || [ ! -r "$codex_first" ]; then
    printf 'missing'
    return 0
  fi
  if grep -q 'AGMSG_CODEX_SHIM_WRAPPER\|agmsg monitor mode' "$codex_first" 2>/dev/null; then
    printf 'agmsg'
  elif grep -q 'CODEX_MONITOR_SHIM_WRAPPER\|Codex monitor shim' "$codex_first" 2>/dev/null; then
    printf 'codex-monitor'
  else
    printf 'none-or-unknown'
  fi
}

agmsg_delivery_mode() {
  "$delivery_script" status codex "$project" 2>/dev/null \
    | awk -F': ' '$1 == "mode" { print $2; exit }'
}

ensure_agmsg_monitor_delivery_mode() {
  local shim_kind delivery_mode
  shim_kind="$(codex_entrypoint_kind)"
  [ "$shim_kind" = "agmsg" ] || return 0

  delivery_mode="$(agmsg_delivery_mode || true)"
  [ -n "$delivery_mode" ] || delivery_mode="unknown"

  section "agmsg delivery mode"
  printf 'codex_shim=agmsg\nmode=%s\n' "$delivery_mode"
  if [ "$delivery_mode" = "monitor" ] || [ "$delivery_mode" = "both" ]; then
    printf 'note=%s\n' 'legacy agmsg monitor mode is active; explicit codex-monitor apply will stop only the same team/name legacy bridge after dry-run.'
  else
    printf 'note=%s\n' 'legacy agmsg monitor mode is not active.'
  fi
}

thread="$(resolve_thread)"
project_hash="$(printf '%s' "$project" | shasum | awk '{print $1}')"
marker_name=""
if [ -n "$thread" ] && [ -f "$agmsg_run_dir/codex-name.$project_hash.$thread" ]; then
  marker_name="$(head -1 "$agmsg_run_dir/codex-name.$project_hash.$thread" 2>/dev/null || true)"
fi

if [ -z "$team" ] && [ -n "${CDXM_MONITOR_TEAM:-}" ]; then team="$CDXM_MONITOR_TEAM"; fi
if [ -z "$name" ] && [ -n "${CDXM_MONITOR_NAME:-}" ]; then name="$CDXM_MONITOR_NAME"; fi
if [ -z "$team" ] && [ -n "${AGMSG_TEAM:-}" ]; then team="$AGMSG_TEAM"; fi
if [ -z "$name" ] && [ -n "${AGMSG_AGENT:-}" ]; then name="$AGMSG_AGENT"; fi
if [ -z "$name" ] && [ -n "${AGMSG_CODEX_NAME:-}" ]; then name="$AGMSG_CODEX_NAME"; fi
if [ -z "$name" ] && [ -n "$marker_name" ]; then name="$marker_name"; fi

if [ -n "$name" ] && [ -z "$team" ]; then
  matching_teams="$(resolve_team_for_name "$name" | paste -sd, -)"
  case "$matching_teams" in
    "")
      printf 'codex-monitor cannot apply: %s is not a registered codex identity for %s.\n' "$name" "$project" >&2
      exit 2
      ;;
    *,*)
      printf 'codex-monitor needs explicit team: %s exists in multiple teams (%s). Specify --team.\n' "$name" "$matching_teams" >&2
      exit 2
      ;;
    *)
      team="$matching_teams"
      ;;
  esac
fi

if [ -z "$team" ] || [ -z "$name" ]; then
  section "agmsg identity"
  whoami_output="$("$whoami_script" "$project" codex)"
  printf '%s\n' "$whoami_output"

  case "$whoami_output" in
    multiple=true*)
      printf 'codex-monitor apply needs the current persona: multiple identities are registered for this cwd, and no session marker/env identified which one this Codex is acting as. Run `/agmsg actas <name>` first, launch with AGMSG_CODEX_NAME=<name>, or pass --team/--name.\n' >&2
      exit 2
      ;;
    suggest=true*)
      printf 'codex-monitor needs an exact joined identity: this cwd is not joined exactly, but other identities were suggested. Join or specify --team/--name explicitly.\n' >&2
      exit 2
      ;;
    not_joined=true*)
      printf 'codex-monitor cannot apply: this cwd is not joined to an agmsg team for codex.\n' >&2
      exit 2
      ;;
  esac

  name="${name:-$(extract_field "$whoami_output" agent)}"
  team="${team:-$(extract_field "$whoami_output" teams)}"
fi

if [ -z "$team" ] || [ -z "$name" ]; then
  printf 'codex-monitor needs team/name: could not resolve the current session persona.\n' >&2
  exit 2
fi

case "$team" in
  *,*)
    printf 'codex-monitor needs explicit team: multiple teams matched (%s). Specify --team explicitly.\n' "$team" >&2
    exit 2
    ;;
esac

ensure_agmsg_monitor_delivery_mode

section "selection"
printf 'cwd=%s\nteam=%s\nname=%s\nmode=%s\ntarget=%s\nthread=%s\n' "$project" "$team" "$name" "$mode" "$target" "${thread:-auto}"

if [ -x "$context_script" ]; then
  "$context_script" "$project" "$team" "$name"
fi

section "doctor"
watch_args=(--team "$team" --name "$name" --cwd "$project" --mode "$mode")
if [ -n "$thread" ]; then
  watch_args+=(--thread "$thread")
fi

doctor_output="$("$codex_monitor_bin" "${target_args[@]}" agmsg doctor "${watch_args[@]}")"
printf '%s\n' "$doctor_output"

target_consumer="$(printf '%s\n' "$doctor_output" | grep -F $'doctor\tconsumer\ttarget' || true)"
monitor_target_consumer="$(printf '%s\n' "$target_consumer" | grep -F 'kind=codex-monitor-agmsg-watch' || true)"
active_monitor_consumer=""
stale_monitor_consumer=""
if [ -n "$monitor_target_consumer" ]; then
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    consumer_thread="$(extract_tab_field "$line" thread)"
    [ "$consumer_thread" = "-" ] && consumer_thread=""
    if [ -z "$thread" ] || [ "$consumer_thread" = "$thread" ]; then
      active_monitor_consumer="${active_monitor_consumer}${line}"$'\n'
    else
      stale_monitor_consumer="${stale_monitor_consumer}${line}"$'\n'
    fi
  done <<<"$monitor_target_consumer"
fi

if [ -n "$active_monitor_consumer" ]; then
  printf 'codex-monitor already has an active target consumer for %s/%s; leaving it in place.\n' "$team" "$name"
  exit 0
fi
if [ -n "$stale_monitor_consumer" ]; then
  stale_thread="$(extract_tab_field "$stale_monitor_consumer" thread)"
  [ "$stale_thread" = "-" ] && stale_thread=""
  printf 'codex-monitor detected a stale thread pin for %s/%s: existing=%s desired=%s. Continuing to dry-run and refresh the LaunchAgent; repair manually with: %s agmsg launch-agent install' "$team" "$name" "${stale_thread:-unpinned}" "${thread:-auto}" "$codex_monitor_bin"
  printf ' %s' "${watch_args[@]}"
  printf ' --force --load\n'
fi

legacy_replace_pending=0
legacy_target_consumer="$(printf '%s\n' "$target_consumer" | grep -F 'kind=codex-bridge' || true)"
other_target_consumer="$(printf '%s\n' "$target_consumer" | grep -v -F 'kind=codex-bridge' | grep -v -F 'kind=codex-monitor-agmsg-watch' || true)"
if [ -n "$legacy_target_consumer" ]; then
  if [ "$replace_legacy" -eq 1 ] && { [ "$apply" -eq 1 ] || [ "$foreground" -eq 1 ]; }; then
    printf 'codex-monitor explicit apply: legacy codex-bridge target consumer for %s/%s will be replaced after dry-run succeeds.\n' "$team" "$name"
    legacy_replace_pending=1
  elif [ "$replace_legacy" -eq 0 ]; then
    printf 'legacy codex-bridge target consumer already receives %s/%s; --no-replace-legacy prevents codex-monitor install.\n' "$team" "$name"
    printf '%s\n' "$legacy_target_consumer"
    exit 0
  else
    printf 'legacy codex-bridge target consumer already receives %s/%s; dry-run continues without replacing it.\n' "$team" "$name"
  fi
fi

if [ -n "$other_target_consumer" ]; then
  printf 'another active target consumer already receives %s/%s; not installing a second monitor to avoid duplicate delivery.\n' "$team" "$name"
  printf '%s\n' "$other_target_consumer"
  exit 0
fi

section "dry run"
"$codex_monitor_bin" "${target_args[@]}" monitor watch agmsg "${watch_args[@]}" --dry-run

if [ "$legacy_replace_pending" -eq 1 ]; then
  section "replace legacy"
  printf 'note=%s\n' 'leaving project-wide legacy agmsg monitor mode unchanged; replacing only the same team/name legacy codex-bridge.'
  stopped="$(stop_legacy_codex_bridge_consumers "$legacy_target_consumer")"
  stopped_current="$(stop_windows_legacy_codex_bridge_consumers)"
  stopped=$((stopped + stopped_current))
  printf 'stopped legacy codex-bridge process(es): %s\n' "$stopped"
  sleep 0.3
fi

if [ "$foreground" -eq 1 ]; then
  section "foreground watch"
  exec "$codex_monitor_bin" "${target_args[@]}" monitor watch agmsg "${watch_args[@]}"
fi

if [ "$apply" -eq 0 ]; then
  platform="$(uname -s 2>/dev/null || printf 'unknown')"
  case "$platform" in
    MINGW*|MSYS*|CYGWIN*)
      printf '\nnext: %s --target %s monitor watch agmsg' "$codex_monitor_bin" "$target"
      printf ' %s' "${watch_args[@]}"
      printf '\n'
      ;;
    *)
      printf '\nnext: %s --target %s agmsg launch-agent install' "$codex_monitor_bin" "$target"
      printf ' %s' "${watch_args[@]}"
      printf ' --force --load\n'
      ;;
  esac
  exit 0
fi

platform="$(uname -s 2>/dev/null || printf 'unknown')"
case "$platform" in
  MINGW*|MSYS*|CYGWIN*)
    section "windows background watch"
    local_appdata="${LOCALAPPDATA:-$HOME/AppData/Local}"
    if command -v cygpath >/dev/null 2>&1; then
      local_appdata="$(cygpath -u "$local_appdata" 2>/dev/null || printf '%s' "$local_appdata")"
    fi
    log_dir="$local_appdata/codex-monitor/logs"
    mkdir -p "$log_dir"
    safe_team="$(printf '%s' "$team" | LC_ALL=C tr -c 'A-Za-z0-9_.-' '_')"
    safe_name="$(printf '%s' "$name" | LC_ALL=C tr -c 'A-Za-z0-9_.-' '_')"
    stdout_log="$log_dir/agmsg-$safe_team-$safe_name.out.log"
    stderr_log="$log_dir/agmsg-$safe_team-$safe_name.err.log"
    nohup "$codex_monitor_bin" "${target_args[@]}" monitor watch agmsg "${watch_args[@]}" >"$stdout_log" 2>"$stderr_log" < /dev/null &
    watch_pid="$!"
    printf 'started_pid=%s\nstdout_log=%s\nstderr_log=%s\n' "$watch_pid" "$stdout_log" "$stderr_log"
    sleep 0.5
    if ! kill -0 "$watch_pid" 2>/dev/null; then
      printf 'codex-monitor background watch exited early; stderr follows:\n' >&2
      tail -50 "$stderr_log" >&2 || true
      exit 1
    fi
    "$codex_monitor_bin" "${target_args[@]}" agmsg doctor "${watch_args[@]}" || true
    exit 0
    ;;
  Darwin*) ;;
  *)
    section "background watch unavailable"
    printf 'durable LaunchAgent install is only available on macOS, and this helper has no background installer for platform=%s.\n' "$platform" >&2
    printf 'Run foreground watch manually: %s --target %s monitor watch agmsg' "$codex_monitor_bin" "$target"
    printf ' %s' "${watch_args[@]}"
    printf '\n'
    exit 1
    ;;
esac

section "launch-agent"
status_output="$("$codex_monitor_bin" "${target_args[@]}" agmsg launch-agent status --team "$team" --name "$name" 2>&1 || true)"
printf '%s\n' "$status_output"
if printf '%s\n' "$status_output" | grep -q 'installed=true' \
  && printf '%s\n' "$status_output" | grep -q 'loaded=true'; then
  status_active_thread="$(extract_tab_field "$status_output" active_thread)"
  status_args_match="$(extract_tab_field "$status_output" args_match)"
  if [ -n "$thread" ] && [ "$status_active_thread" != "$thread" ]; then
    printf 'codex-monitor LaunchAgent is loaded for %s/%s but has stale active_thread=%s desired=%s; reinstalling with --force --load.\n' "$team" "$name" "${status_active_thread:-unknown}" "$thread"
  elif [ "$status_args_match" = "false" ]; then
    printf 'codex-monitor LaunchAgent plist and active arguments differ for %s/%s; reinstalling with --force --load.\n' "$team" "$name"
  else
  printf 'codex-monitor LaunchAgent already installed and loaded for %s/%s.\n' "$team" "$name"
  exit 0
  fi
fi

"$codex_monitor_bin" "${target_args[@]}" agmsg launch-agent install "${watch_args[@]}" --force --load
"$codex_monitor_bin" "${target_args[@]}" agmsg launch-agent status --team "$team" --name "$name"
