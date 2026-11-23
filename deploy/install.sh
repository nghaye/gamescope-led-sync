#!/usr/bin/env bash
# install.sh — idempotent install/uninstall of gamescope-led-sync (SPEC §9).
#
#   install.sh [--binary PATH] [--enable]
#   install.sh --uninstall
#
# Without --binary, builds with `cargo build --release --locked` when cargo is
# available. Installs the binary to ~/.local/bin/gamescope-led-sync (plain
# install -m 0755, no sudo), copies deploy/config.toml.example to
# ~/.config/gamescope-led-sync/config.toml only if absent (never overwrites a
# user config), installs the systemd user unit to ~/.config/systemd/user/ and
# runs `systemctl --user daemon-reload`. --enable additionally runs
# `systemctl --user enable --now gamescope-led-sync`.
#
# --uninstall disables and stops the unit, removes the unit and the binary,
# and reloads the user daemon. The user config file is kept.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY_NAME="gamescope-led-sync"
UNIT_NAME="gamescope-led-sync.service"
CONFIG_DIR="$HOME/.config/gamescope-led-sync"
UNIT_DIR="$HOME/.config/systemd/user"
LOCAL_BINARY="$HOME/.local/bin/$BINARY_NAME"

usage() {
  cat <<EOF
Usage: install.sh [--binary PATH] [--enable]
       install.sh --uninstall

  --binary PATH   install this prebuilt binary instead of building with cargo
  --enable        systemctl --user enable --now $UNIT_NAME after installing
  --uninstall     disable --now, remove the unit and the binary (config kept)
EOF
}

BINARY=""
ENABLE=0
UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --binary)
      [ $# -ge 2 ] || { echo "--binary needs a path" >&2; exit 2; }
      BINARY="$2"
      shift 2
      ;;
    --binary=*)
      BINARY="${1#--binary=}"
      shift
      ;;
    --enable) ENABLE=1; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ "$UNINSTALL" -eq 1 ]; then
  systemctl --user disable --now "$UNIT_NAME" 2>/dev/null || true
  rm -f "$UNIT_DIR/$UNIT_NAME"
  rm -f "$LOCAL_BINARY"
  systemctl --user daemon-reload
  echo "uninstalled $UNIT_NAME and $LOCAL_BINARY"
  echo "the config in $CONFIG_DIR/config.toml was kept"
  exit 0
fi

if [ -z "$BINARY" ]; then
  if command -v cargo >/dev/null 2>&1; then
    (cd "$REPO_DIR" && cargo build --release --locked)
    BINARY="$REPO_DIR/target/release/$BINARY_NAME"
  else
    echo "no --binary given and cargo is not available" >&2
    exit 1
  fi
fi
if [ ! -f "$BINARY" ]; then
  echo "binary not found: $BINARY" >&2
  exit 1
fi

mkdir -p "$HOME/.local/bin"
install -m 0755 "$BINARY" "$LOCAL_BINARY"

# Config: copy the example only if absent — never overwrite a user config.
mkdir -p "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/config.toml" ]; then
  install -m 0644 "$SCRIPT_DIR/config.toml.example" "$CONFIG_DIR/config.toml"
  echo "installed example config to $CONFIG_DIR/config.toml"
else
  echo "keeping existing $CONFIG_DIR/config.toml"
fi

# systemd user unit.
mkdir -p "$UNIT_DIR"
install -m 0644 "$SCRIPT_DIR/$UNIT_NAME" "$UNIT_DIR/$UNIT_NAME"
systemctl --user daemon-reload

if [ "$ENABLE" -eq 1 ]; then
  systemctl --user enable --now "$UNIT_NAME"
fi

echo "installed $LOCAL_BINARY and $UNIT_DIR/$UNIT_NAME"
echo "enable at boot + start now: systemctl --user enable --now $UNIT_NAME"
echo "logs: journalctl --user -u $UNIT_NAME -f"
