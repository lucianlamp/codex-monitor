#!/usr/bin/env bash
set -euo pipefail

# Codex monitor shim (shared across macOS, Linux and Windows/Git Bash).
#
# Installed as the `codex` entrypoint ahead of the real Codex binary on PATH.
# For interactive Codex launches it starts (or reuses) a loopback ws://
# app-server and execs the real `codex --remote ws://127.0.0.1:<port>`, so
# codex-monitor can observe/deliver on the same app-server. Everything in the
# passthrough list is forwarded to the real Codex unchanged.
#
# On Windows this is invoked through a thin codex.cmd launcher
# (`bash -l <this script>`), so the logic stays identical on every platform.
# CODEX_MONITOR_SHIM_WRAPPER=1
export CODEX_MONITOR_SHIM_WRAPPER=1

# A `codex` found on PATH may itself be another wrapper shim (codex-monitor or
# agmsg) rather than the real binary. Chaining shim into shim hides the real
# app-server binding, so skip any candidate that carries a known shim marker.
# Only the head is scanned, so a large real codex binary stays cheap to reject.
is_codex_shim() {
  case "$(head -c 2 "$1" 2>/dev/null || true)" in
    '#!') ;;
    *) return 1 ;;
  esac
  case "$(head -c 4096 "$1" 2>/dev/null || true)" in
    *CODEX_MONITOR_SHIM_WRAPPER*|*AGMSG_CODEX_SHIM_WRAPPER*) return 0 ;;
    *) return 1 ;;
  esac
}

resolve_real_codex() {
  if [ -n "${CODEX_MONITOR_REAL_CODEX:-}" ]; then
    printf '%s\n' "$CODEX_MONITOR_REAL_CODEX"
    return 0
  fi

  local self_dir self_path shim_target old_ifs path_dir candidate candidate_dir candidate_path
  self_dir="$(cd "$(dirname "$0")" && pwd)"
  self_path="$self_dir/$(basename "$0")"
  shim_target="${CODEX_MONITOR_SHIM_TARGET:-$self_path}"

  old_ifs="$IFS"
  IFS=:
  for path_dir in $PATH; do
    IFS="$old_ifs"
    [ -n "$path_dir" ] || path_dir="."
    candidate="$path_dir/codex"
    if [ -x "$candidate" ]; then
      candidate_dir="$(cd "$(dirname "$candidate")" 2>/dev/null && pwd || true)"
      [ -n "$candidate_dir" ] || continue
      candidate_path="$candidate_dir/$(basename "$candidate")"
      if [ "$candidate_path" != "$self_path" ] && [ "$candidate_path" != "$shim_target" ] \
        && ! is_codex_shim "$candidate_path"; then
        printf '%s\n' "$candidate_path"
        return 0
      fi
    fi
    IFS=:
  done
  IFS="$old_ifs"

  echo "codex-monitor shim: real codex not found on PATH" >&2
  return 1
}

project_from_args() {
  local project="$PWD"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --cd|--cwd|-C)
        if [ "$#" -gt 1 ]; then
          project="$2"
          shift 2
          continue
        fi
        ;;
      --cd=*|--cwd=*)
        project="${1#*=}"
        shift
        continue
        ;;
    esac
    shift
  done
  if [ -d "$project" ]; then
    (cd "$project" && pwd)
  else
    printf '%s\n' "$PWD"
  fi
}

first_non_option() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --cd|--cwd|-C)
        shift 2 || true
        ;;
      --cd=*|--cwd=*)
        shift
        ;;
      --help|--version|-h|-V)
        printf '%s\n' "$1"
        return 0
        ;;
      --*)
        shift
        ;;
      -*)
        shift
        ;;
      *)
        printf '%s\n' "$1"
        return 0
        ;;
    esac
  done
  return 1
}

hash_project() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum | awk '{print $1}'
  else
    printf '%s' "$1" | cksum | awk '{print $1}'
  fi
}

port_alive() {
  (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}

shim_run_dir() {
  if [ -n "${CODEX_MONITOR_SHIM_RUN_DIR:-}" ]; then
    printf '%s\n' "$CODEX_MONITOR_SHIM_RUN_DIR"
    return 0
  fi
  case "$(uname -s 2>/dev/null || echo unknown)" in
    Darwin) printf '%s\n' "$HOME/Library/Caches/codex-monitor/shim" ;;
    *) printf '%s\n' "${XDG_CACHE_HOME:-$HOME/.cache}/codex-monitor/shim" ;;
  esac
}

ensure_app_server() {
  local real_codex="$1"
  local project="$2"
  local run_dir project_hash server_log server_pid port_file existing_port existing_pid port
  run_dir="$(shim_run_dir)"

  project_hash="$(hash_project "$project")"
  mkdir -p "$run_dir"
  server_log="$run_dir/codex-app-server.$project_hash.log"
  server_pid="$run_dir/codex-app-server.$project_hash.pid"
  port_file="$run_dir/codex-app-server.$project_hash.port"

  port=""
  if [ -f "$port_file" ] && [ -f "$server_pid" ]; then
    existing_port="$(cat "$port_file" 2>/dev/null || true)"
    existing_pid="$(cat "$server_pid" 2>/dev/null || true)"
    if [ -n "$existing_port" ] && [ -n "$existing_pid" ] \
      && kill -0 "$existing_pid" 2>/dev/null && port_alive "$existing_port"; then
      port="$existing_port"
    fi
  fi

  if [ -z "$port" ]; then
    : > "$server_log"
    # Merge stderr into the log: codex prints "listening on: ws://..." to stderr.
    "$real_codex" app-server --listen "ws://127.0.0.1:0" >>"$server_log" 2>&1 &
    echo "$!" > "$server_pid"
    for _ in $(seq 1 100); do
      port="$(sed -n 's#.*listening on: ws://127\.0\.0\.1:\([0-9][0-9]*\).*#\1#p' "$server_log" | head -1)"
      [ -n "$port" ] && break
      sleep 0.1
    done
    if [ -z "$port" ]; then
      echo "codex-monitor shim: app-server did not report a listening port" >&2
      echo "codex-monitor shim: see $server_log" >&2
      exit 1
    fi
    printf '%s' "$port" > "$port_file"
  fi

  if ! port_alive "$port"; then
    echo "codex-monitor shim: app-server did not start on ws://127.0.0.1:$port" >&2
    echo "codex-monitor shim: see $server_log" >&2
    exit 1
  fi
  printf 'ws://127.0.0.1:%s' "$port"
}

real_codex="$(resolve_real_codex)"

if [ "${CODEX_MONITOR_SHIM_DISABLE:-}" = "1" ]; then
  exec "$real_codex" "$@"
fi

command_name="$(first_non_option "$@" || true)"
case "$command_name" in
  app-server|exec|login|logout|mcp|completion|debug|apply|review|sandbox|help|--help|-h|version|--version|-V)
    exec "$real_codex" "$@"
    ;;
esac

project="$(project_from_args "$@")"
socket_url="$(ensure_app_server "$real_codex" "$project")"

case "$command_name" in
  resume)
    monitor_args=()
    removed_resume=0
    for arg in "$@"; do
      if [ "$removed_resume" -eq 0 ] && [ "$arg" = "resume" ]; then
        removed_resume=1
        continue
      fi
      monitor_args+=("$arg")
    done
    cd "$project"
    exec "$real_codex" resume --remote "$socket_url" "${monitor_args[@]}"
    ;;
  *)
    cd "$project"
    exec "$real_codex" --remote "$socket_url" "$@"
    ;;
esac
