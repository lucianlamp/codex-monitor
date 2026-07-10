#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'usage: cdxm-agmsg-foreground.sh <team> <agent>\n' >&2
  exit 64
fi

team="$1"
agent="$2"
scripts_dir="${AGMSG_SCRIPTS_DIR:-$HOME/.agents/skills/agmsg/scripts}"
inbox="$scripts_dir/inbox.sh"
interval="${CDXM_FOREGROUND_POLL_SECONDS:-2}"

[[ -x "$inbox" ]] || {
  printf 'agmsg inbox script is missing or not executable: %s\n' "$inbox" >&2
  exit 69
}
[[ "$interval" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
  printf 'CDXM_FOREGROUND_POLL_SECONDS must be a non-negative number\n' >&2
  exit 64
}

while :; do
  output="$($inbox "$team" "$agent")"
  normalized="${output//$'\r'/}"
  if [[ -n "${normalized//[[:space:]]/}" && "$normalized" != "No new messages." ]]; then
    printf '%s\n' "$output"
    exit 0
  fi
  sleep "$interval"
done
