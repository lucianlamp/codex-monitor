#!/usr/bin/env bash
set -euo pipefail

REPO_URL="${CDXM_INSTALL_REPO_URL:-https://github.com/lucianlamp/codex-monitor}"
REF="${CDXM_INSTALL_REF:-main}"
INSTALL_ROOT="${CDXM_INSTALL_ROOT:-$HOME/.codex-monitor}"
BIN_DIR="$INSTALL_ROOT/bin"
SKILL_DIR="${CDXM_SKILL_DIR:-$HOME/.codex/skills/codex-monitor}"
AGENTS_BIN="${CDXM_AGENTS_BIN:-$HOME/.agents/bin}"
SHIM_TARGET="$AGENTS_BIN/codex"

RELEASE_BASE="${CDXM_INSTALL_RELEASE_BASE:-https://github.com/lucianlamp/codex-monitor/releases/latest/download}"
BUILD_FROM_SOURCE=0

ASSUME_YES=0
INSTALL_SHIM=""
UPDATE_PATH=""
SKIP_BUILD=0
SOURCE_DIR=""
TMP_DIR=""

usage() {
  cat <<'EOF'
Usage: install.sh [options]

Installs codex-monitor for daily use:
  - cdxm and codex-monitor binaries under ~/.codex-monitor/bin
  - the Codex skill under ~/.codex/skills/codex-monitor
  - an optional Codex CLI shim at ~/.agents/bin/codex

Options:
  --yes             Accept default yes prompts for binary, skill, and PATH.
  --install-shim    Install the Codex shim if ~/.agents/bin/codex is absent.
  --no-shim         Skip Codex shim installation.
  --no-path              Do not update shell PATH files.
  --source <path>        Use a local codex-monitor checkout instead of downloading.
  --skip-build           Install nothing: skip the prebuilt download and the cargo build. Useful for installer tests.
  --build-from-source    Skip prebuilt download; always build from source with cargo.
  --help                 Show this help.

With --install-shim the installer backs up an existing ~/.agents/bin/codex
before replacing it with the codex-monitor shim.
On macOS, prebuilt binaries are downloaded by default. Use --build-from-source
to build with cargo instead.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes|-y)
      ASSUME_YES=1
      shift
      ;;
    --install-shim)
      INSTALL_SHIM=1
      shift
      ;;
    --no-shim)
      INSTALL_SHIM=0
      shift
      ;;
    --no-path)
      UPDATE_PATH=0
      shift
      ;;
    --source)
      SOURCE_DIR="${2:?--source requires a path}"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --build-from-source)
      BUILD_FROM_SOURCE=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cleanup() {
  if [ -n "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

prompt_yes_no() {
  local prompt="$1"
  local default="$2"
  local answer suffix

  if [ "$ASSUME_YES" -eq 1 ]; then
    case "$default" in
      y|Y) return 0 ;;
      *) return 1 ;;
    esac
  fi

  case "$default" in
    y|Y) suffix="[Y/n]" ;;
    *) suffix="[y/N]" ;;
  esac

  if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    printf '%s %s ' "$prompt" "$suffix" >/dev/tty
    IFS= read -r answer </dev/tty || answer=""
  else
    answer=""
  fi

  answer="${answer:-$default}"
  case "$answer" in
    y|Y|yes|YES|Yes) return 0 ;;
    *) return 1 ;;
  esac
}

repo_from_script_dir() {
  local self="${BASH_SOURCE[0]:-}"
  local dir
  if [ -n "$self" ] && [ -f "$self" ]; then
    dir="$(cd "$(dirname "$self")" && pwd)"
    if [ -f "$dir/Cargo.toml" ] && [ -d "$dir/skills/codex-monitor" ]; then
      printf '%s\n' "$dir"
      return 0
    fi
  fi
  return 1
}

download_source() {
  local archive_url="$REPO_URL/archive/refs/heads/$REF.tar.gz"
  TMP_DIR="$(mktemp -d)"
  echo "Downloading codex-monitor from $archive_url"
  curl -fsSL "$archive_url" | tar -xz -C "$TMP_DIR"
  find "$TMP_DIR" -maxdepth 1 -type d -name "codex-monitor-*" | head -1
}

resolve_source_dir() {
  if [ -n "$SOURCE_DIR" ]; then
    (cd "$SOURCE_DIR" && pwd)
    return 0
  fi
  if repo_from_script_dir; then
    return 0
  fi
  download_source
}

prebuilt_target() {
  # Only macOS has prebuilt archives; everything else returns empty.
  [ "$(uname -s)" = "Darwin" ] || return 0
  case "$(uname -m)" in
    arm64|aarch64) printf 'aarch64-apple-darwin\n' ;;
    x86_64) printf 'x86_64-apple-darwin\n' ;;
  esac
}

download_prebuilt() {
  local target="$1"
  local archive="codex-monitor-$target.tar.gz"
  local url="$RELEASE_BASE/$archive"
  local dl_dir expected actual
  dl_dir="$(mktemp -d)"

  echo "Downloading prebuilt binaries: $url"
  if ! curl -fsSL "$url" -o "$dl_dir/$archive"; then
    echo "Prebuilt download failed; falling back to source build." >&2
    rm -rf "$dl_dir"
    return 1
  fi
  # Integrity is mandatory: a missing checksum is fail-safe (fall back to a
  # source build), never fail-open (install an unverified binary). Only a
  # checksum that is present AND mismatches is treated as active tampering and
  # aborts the whole installer.
  if ! curl -fsSL "$url.sha256" -o "$dl_dir/$archive.sha256"; then
    echo "No published checksum for $archive; refusing to install an unverified binary. Falling back to source build." >&2
    rm -rf "$dl_dir"
    return 1
  fi
  expected="$(tr -d '[:space:]' < "$dl_dir/$archive.sha256")"
  actual="$(shasum -a 256 "$dl_dir/$archive" | awk '{print $1}')"
  if [ "$expected" != "$actual" ]; then
    echo "Checksum mismatch for $archive (expected $expected, got $actual)" >&2
    rm -rf "$dl_dir"
    exit 1
  fi

  mkdir -p "$BIN_DIR"
  tar -xzf "$dl_dir/$archive" -C "$BIN_DIR" codex-monitor cdxm
  chmod +x "$BIN_DIR/codex-monitor" "$BIN_DIR/cdxm"
  xattr -d com.apple.quarantine "$BIN_DIR/codex-monitor" "$BIN_DIR/cdxm" 2>/dev/null || true
  rm -rf "$dl_dir"
  echo "Installed prebuilt cdxm to $BIN_DIR/cdxm"
  return 0
}

install_binaries() {
  if [ "$SKIP_BUILD" -eq 1 ]; then
    mkdir -p "$BIN_DIR"
    echo "Skipped binary install (--skip-build)."
    return 0
  fi

  if [ "$BUILD_FROM_SOURCE" -eq 0 ]; then
    local target
    target="$(prebuilt_target)"
    if [ -n "$target" ] && download_prebuilt "$target"; then
      return 0
    fi
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to build codex-monitor from source." >&2
    echo "Install Rust/Cargo, then rerun this installer." >&2
    exit 1
  fi

  mkdir -p "$INSTALL_ROOT"
  cargo install --path "$SOURCE_DIR" --bins --force --root "$INSTALL_ROOT"
  echo "Installed cdxm to $BIN_DIR/cdxm"
}

install_skill() {
  local source_skill="$SOURCE_DIR/skills/codex-monitor"
  if [ ! -d "$source_skill" ]; then
    echo "missing skill source: $source_skill" >&2
    exit 1
  fi
  mkdir -p "$(dirname "$SKILL_DIR")"
  rm -rf "$SKILL_DIR"
  cp -R "$source_skill" "$SKILL_DIR"
  chmod +x "$SKILL_DIR"/scripts/*.sh 2>/dev/null || true
  echo "Installed Codex skill to $SKILL_DIR"
}

shim_kind() {
  local path="$1"
  if [ ! -f "$path" ]; then
    echo "missing"
  elif grep -q "CODEX_MONITOR_SHIM_WRAPPER=1" "$path" 2>/dev/null; then
    echo "codex-monitor"
  elif grep -Eq "AGMSG_CODEX_SHIM_WRAPPER|agmsg monitor mode|codex-shim.sh" "$path" 2>/dev/null; then
    echo "agmsg"
  else
    echo "custom-or-unknown"
  fi
}

write_codex_shim() {
  mkdir -p "$AGENTS_BIN"

  local source_shim="$SOURCE_DIR/skills/codex-monitor/scripts/codex-shim.sh"
  if [ ! -f "$source_shim" ]; then
    echo "missing shim source: $source_shim" >&2
    exit 1
  fi

  if [ -e "$SHIM_TARGET" ]; then
    local kind
    kind="$(shim_kind "$SHIM_TARGET")"
    if [ "$kind" = "codex-monitor" ]; then
      cp "$source_shim" "$SHIM_TARGET"
      chmod +x "$SHIM_TARGET"
      echo "Refreshed existing codex-monitor shim at $SHIM_TARGET"
      return 0
    fi
    # write_codex_shim only runs when the shim install was explicitly requested,
    # so take over a foreign entrypoint -- but keep a timestamped backup first.
    local stamp backup
    stamp="$(date +%Y%m%d-%H%M%S)"
    backup="$SHIM_TARGET.bak-$stamp"
    cp "$SHIM_TARGET" "$backup"
    echo "Backed up existing $kind codex entrypoint to $backup"
  fi

  cp "$source_shim" "$SHIM_TARGET"
  chmod +x "$SHIM_TARGET"
  echo "Installed Codex monitor shim to $SHIM_TARGET"
}

update_path_file() {
  local rc_file="$HOME/.zshrc"
  local marker_begin="# >>> codex-monitor PATH >>>"
  local marker_end="# <<< codex-monitor PATH <<<"
  local tmp path_entry
  mkdir -p "$(dirname "$rc_file")"
  if [ -f "$rc_file" ] && grep -qF "$marker_begin" "$rc_file"; then
    tmp="$(mktemp)"
    awk -v begin="$marker_begin" -v end="$marker_end" '
      $0 == begin { skip = 1; next }
      $0 == end { skip = 0; next }
      !skip { print }
    ' "$rc_file" > "$tmp"
    mv "$tmp" "$rc_file"
  fi
  {
    printf '\n%s\n' "$marker_begin"
    for path_entry in "$@"; do
      printf 'export PATH="%s:$PATH"\n' "$path_entry"
    done
    printf '%s\n' "$marker_end"
  } >> "$rc_file"
  echo "Managed codex-monitor PATH entries in $rc_file"
}

SOURCE_DIR="$(resolve_source_dir)"
if [ ! -f "$SOURCE_DIR/Cargo.toml" ]; then
  echo "source does not look like codex-monitor: $SOURCE_DIR" >&2
  exit 1
fi

echo "codex-monitor installer"
echo "source: $SOURCE_DIR"
echo "binary root: $INSTALL_ROOT"
echo "skill dir: $SKILL_DIR"
echo "codex shim target: $SHIM_TARGET"

if prompt_yes_no "Install cdxm and codex-monitor binaries to $BIN_DIR?" y; then
  install_binaries
else
  echo "Skipped binary install."
fi

if prompt_yes_no "Install Codex skill to $SKILL_DIR?" y; then
  install_skill
else
  echo "Skipped skill install."
fi

if [ -z "$INSTALL_SHIM" ]; then
  if prompt_yes_no "Install Codex shim at $SHIM_TARGET?" n; then
    INSTALL_SHIM=1
  else
    INSTALL_SHIM=0
  fi
fi

if [ "$INSTALL_SHIM" = "1" ]; then
  write_codex_shim
else
  echo "Skipped Codex shim install."
fi

if [ -z "$UPDATE_PATH" ]; then
  if prompt_yes_no "Add $BIN_DIR and $AGENTS_BIN to PATH in ~/.zshrc?" y; then
    UPDATE_PATH=1
  else
    UPDATE_PATH=0
  fi
fi

if [ "$UPDATE_PATH" = "1" ]; then
  update_path_file "$BIN_DIR" "$AGENTS_BIN"
else
  echo "Skipped PATH update."
fi

echo ""
echo "Done."
echo "Open a new shell or run: source ~/.zshrc"
echo "Then verify:"
echo "  command -v cdxm"
echo "  type -a codex"
