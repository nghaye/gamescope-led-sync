#!/usr/bin/env bash
# install-service.sh — idempotent install/uninstall of the gamescope-led-sync
# service + binary, user-space only (SPEC §9/§11). This is the old
# deploy/install.sh, moved to scripts/ when the Decky plugin became the main
# install entry point; the artifacts it installs live in deploy/ and are
# embedded into the plugin's payload by scripts/package-plugin.sh.
#
#   install-service.sh [--binary PATH] [--enable]
#   install-service.sh --uninstall
#
# Without --binary, builds with `cargo build --release --locked` when cargo is
# available. Installs the binary to ~/.local/bin/gamescope-led-sync (plain
# install -m 0755, no sudo), copies deploy/config.toml.example to
# ~/.config/gamescope-led-sync/config.toml only if absent (never overwrites a
# user config), installs the systemd user unit to ~/.config/systemd/user/,
# installs the calibration launcher (deploy/gamescope-led-sync-calibrate.desktop
# with @BIN@ replaced by the installed binary path) to
# ~/.local/share/applications/ so it can be added as a non-Steam game in Steam
# Big Picture, and runs `systemctl --user daemon-reload`. --enable additionally
# runs `systemctl --user enable --now gamescope-led-sync`.
#
# --uninstall disables and stops the unit, removes the unit, the binary, and
# the calibration .desktop, and reloads the user daemon. The user config file
# is kept.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DEPLOY_DIR="$REPO_DIR/deploy"
BINARY_NAME="gamescope-led-sync"
UNIT_NAME="gamescope-led-sync.service"
DESKTOP_NAME="gamescope-led-sync-calibrate.desktop"
CONFIG_DIR="$HOME/.config/gamescope-led-sync"
UNIT_DIR="$HOME/.config/systemd/user"
APP_DIR="$HOME/.local/share/applications"
LOCAL_BINARY="$HOME/.local/bin/$BINARY_NAME"

usage() {
  cat <<EOF
Usage: install-service.sh [--binary PATH] [--enable]
       install-service.sh --uninstall

  --binary PATH   install this prebuilt binary instead of building with cargo
  --enable        systemctl --user enable --now $UNIT_NAME after installing
  --uninstall     disable --now, remove the unit, the binary and the
                  calibration .desktop (config kept)
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
  rm -f "$APP_DIR/$DESKTOP_NAME"
  systemctl --user daemon-reload
  echo "uninstalled $UNIT_NAME, $LOCAL_BINARY and $APP_DIR/$DESKTOP_NAME"
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
  install -m 0644 "$DEPLOY_DIR/config.toml.example" "$CONFIG_DIR/config.toml"
  echo "installed example config to $CONFIG_DIR/config.toml"
else
  echo "keeping existing $CONFIG_DIR/config.toml"
fi

# systemd user unit.
mkdir -p "$UNIT_DIR"
install -m 0644 "$DEPLOY_DIR/$UNIT_NAME" "$UNIT_DIR/$UNIT_NAME"

# Calibration launcher for Steam Big Picture: the .desktop ships with a @BIN@
# placeholder, filled with the installed binary's absolute path.
mkdir -p "$APP_DIR"
sed "s|@BIN@|$LOCAL_BINARY|g" "$DEPLOY_DIR/$DESKTOP_NAME" > "$APP_DIR/$DESKTOP_NAME"
chmod 0644 "$APP_DIR/$DESKTOP_NAME"
echo "installed $APP_DIR/$DESKTOP_NAME (add it as a non-Steam game in Steam to launch the calibration from Big Picture)"

systemctl --user daemon-reload

if [ "$ENABLE" -eq 1 ]; then
  systemctl --user enable --now "$UNIT_NAME"
fi

echo "installed $LOCAL_BINARY, $UNIT_DIR/$UNIT_NAME and $APP_DIR/$DESKTOP_NAME"
echo "enable at boot + start now: systemctl --user enable --now $UNIT_NAME"
echo "logs: journalctl --user -u $UNIT_NAME -f"
