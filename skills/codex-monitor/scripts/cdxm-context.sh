#!/bin/sh
set -eu

project="${1:-$(pwd)}"
team="${2:-}"
name="${3:-}"
codex_monitor_bin="${CODEX_MONITOR_BIN:-}"

if [ -z "$codex_monitor_bin" ]; then
  if command -v cdxm >/dev/null 2>&1; then
    codex_monitor_bin="$(command -v cdxm)"
  fi
fi

section() {
  printf '\n[%s]\n' "$1"
}

run() {
  printf '$ %s\n' "$*"
  "$@" || true
}

section "codex-monitor binary"
if [ -n "$codex_monitor_bin" ]; then
  printf '%s\n' "$codex_monitor_bin"
  "$codex_monitor_bin" --help | sed -n '1,24p'
else
  printf 'cdxm not found on PATH\n'
fi

section "codex CLI entrypoint"
codex_first="$(command -v codex 2>/dev/null || true)"
agents_bin="$HOME/.agents/bin"
printf 'codex_first=%s\n' "${codex_first:-missing}"
case ":$PATH:" in
  *":$agents_bin:"*) printf 'agents_bin_on_path=true\n' ;;
  *) printf 'agents_bin_on_path=false\n' ;;
esac
if [ "$codex_first" = "$agents_bin/codex" ]; then
  printf 'agents_bin_precedence=first-codex\n'
else
  printf 'agents_bin_precedence=not-first-codex\n'
fi
if [ -n "$codex_first" ] && [ -r "$codex_first" ]; then
  if grep -q 'AGMSG_CODEX_SHIM_WRAPPER\|codex-shim.sh\|agmsg monitor mode' "$codex_first" 2>/dev/null; then
    printf 'codex_shim=agmsg\n'
  elif grep -q 'codex-monitor' "$codex_first" 2>/dev/null; then
    printf 'codex_shim=codex-monitor\n'
  else
    printf 'codex_shim=none-or-unknown\n'
  fi
else
  printf 'codex_shim=missing\n'
fi
printf 'cli_monitor_note=%s\n' 'Codex CLI sessions should be app-server-bound via a codex shim/PATH entrypoint or explicit codex --remote; plain real-codex TUI launches are not injectable by cwd.'

section "targets"
if [ -n "$codex_monitor_bin" ]; then
  run "$codex_monitor_bin" targets
fi

if [ -n "$team" ] && [ -n "$name" ]; then
  section "agmsg doctor"
  if [ -n "$codex_monitor_bin" ]; then
    run "$codex_monitor_bin" agmsg doctor --team "$team" --name "$name" --cwd "$project"
  fi
fi

section "app threads for cwd"
if [ -n "$codex_monitor_bin" ]; then
  run "$codex_monitor_bin" --target app threads --cwd "$project"
fi

section "app loaded (hidden diagnostic)"
if [ -n "$codex_monitor_bin" ]; then
  run "$codex_monitor_bin" --target app loaded
fi

section "agmsg identity"
agmsg_scripts="${AGMSG_SCRIPTS_DIR:-$HOME/.agents/skills/agmsg/scripts}"
if [ -x "$agmsg_scripts/whoami.sh" ]; then
  run "$agmsg_scripts/whoami.sh" "$project" codex
fi
if [ -x "$agmsg_scripts/delivery.sh" ]; then
  run "$agmsg_scripts/delivery.sh" status codex "$project"
  run "$agmsg_scripts/delivery.sh" status
fi

if [ -n "$team" ] && [ -n "$name" ]; then
  section "launch-agent status"
  if [ -n "$codex_monitor_bin" ]; then
    run "$codex_monitor_bin" agmsg launch-agent status --team "$team" --name "$name"
  fi
fi
