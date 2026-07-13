#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  printf 'usage: cdxm-agmsg-foreground.sh <team> <agent> [raw-pending-path]\n' >&2
  exit 64
fi

team="$1"
agent="$2"
raw_pending_path="${3:-}"
scripts_dir="${AGMSG_SCRIPTS_DIR:-$HOME/.agents/skills/agmsg/scripts}"
inbox="$scripts_dir/inbox.sh"
interval="${CDXM_FOREGROUND_POLL_SECONDS:-2}"
owner_pid="${CDXM_FOREGROUND_PARENT_PID:-}"
umask 077
if [[ -z "$raw_pending_path" ]]; then
  raw_pending_path="$(mktemp)"
  trap 'rm -f "$raw_pending_path"' EXIT
fi

[[ -x "$inbox" ]] || {
  printf 'agmsg inbox script is missing or not executable: %s\n' "$inbox" >&2
  exit 69
}
[[ "$interval" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
  printf 'CDXM_FOREGROUND_POLL_SECONDS must be a non-negative number\n' >&2
  exit 64
}

while :; do
  if [[ -n "$owner_pid" ]] && ! kill -0 "$owner_pid" 2>/dev/null; then
    exit 0
  fi
  "$inbox" "$team" "$agent" > "$raw_pending_path"
  output="$(<"$raw_pending_path")"
  normalized="${output//$'\r'/}"
  if [[ -n "${normalized//[[:space:]]/}" && "$normalized" != "No new messages." ]]; then
    printf '%s\n' "$output"
    exit 0
  fi
  sleep "$interval"
done
