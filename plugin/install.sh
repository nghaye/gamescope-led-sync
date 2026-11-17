#!/usr/bin/env bash
# install.sh — idempotent install/uninstall of the Decky Loader plugin (SPEC §10).
#
#   install.sh [--build] [--direct]
#   install.sh --uninstall [--direct]
#
# Default mode talks to Decky Loader's own local API (plugin/decky-api.mjs):
# GET 127.0.0.1:1337/auth/token is unauthenticated on localhost, and the /ws
# websocket lets us call utilities/install_plugin with a file:// zip — the
# loader (running as root) unzips into ~/homebrew/plugins, fixes ownership and
# imports the plugin itself. No sudo, no loader restart, run as the normal
# user. --uninstall uses utilities/uninstall_plugin the same way.
#
# --direct instead copies the files to ~/homebrew/plugins itself and restarts
# the plugin_loader system service — that needs sudo, because the plugins dir
# is root-owned and the loader is a root unit. Useful when the loader API is
# unreachable.
#
# Build (unless dist/index.js already exists, or forced with --build):
# `npm ci` when package-lock.json is present, else `npm install`, then
# `npm run build`. Runs as the invoking user — never as root, so node_modules
# stays user-owned.
#
# The installed directory is ~/homebrew/plugins/gamescope-led-sync (DECKY_HOME
# overrides the Decky home). The served URLs use the plugin.json display name
# ("Gamescope LED Sync"), see src/index.tsx.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR_NAME="gamescope-led-sync"
DECKY_HOME="${DECKY_HOME:-$HOME/homebrew}"
PLUGINS_DIR="$DECKY_HOME/plugins"
DEST="$PLUGINS_DIR/$PLUGIN_DIR_NAME"
LOADER_UNIT="plugin_loader"
DAEMON_BINARY="/usr/bin/$PLUGIN_DIR_NAME"

usage() {
  cat <<EOF
Usage: install.sh [--build] [--direct]
       install.sh --uninstall [--direct]

  --build      rebuild the frontend even if dist/index.js already exists
  --direct     copy to ~/homebrew/plugins with sudo + restart plugin_loader
               (default: install through the loader's local API, no sudo)
  --uninstall  remove the plugin (its data in $DECKY_HOME/data and loader
               settings are kept by the loader)

Environment:  DECKY_HOME   Decky Loader home (default: ~/homebrew)
EOF
}

BUILD=0
DIRECT=0
UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --build) BUILD=1; shift ;;
    --direct) DIRECT=1; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

PLUGIN_NAME="$(node -p "JSON.parse(require('fs').readFileSync('$SCRIPT_DIR/plugin.json','utf8')).name")"

if [ ! -d "$PLUGINS_DIR" ] && [ "$DIRECT" -eq 1 ]; then
  echo "Decky Loader not found: $PLUGINS_DIR does not exist" >&2
  echo "install Decky Loader first (https://github.com/SteamDeckHomebrew/decky-loader)" >&2
  echo "or set DECKY_HOME to your Decky home" >&2
  exit 1
fi

# Build the frontend when needed (as the invoking user, never root).
if [ "$BUILD" -eq 1 ] || [ ! -f "$SCRIPT_DIR/dist/index.js" ]; then
  command -v node >/dev/null 2>&1 || { echo "node is required to build the plugin" >&2; exit 1; }
  command -v npm >/dev/null 2>&1 || { echo "npm is required to build the plugin" >&2; exit 1; }
  echo "building the plugin frontend..."
  (cd "$SCRIPT_DIR"
   if [ -f package-lock.json ]; then npm ci; else npm install; fi
   npm run build)
fi
if [ ! -f "$SCRIPT_DIR/dist/index.js" ]; then
  echo "dist/index.js not found after build (run with --build)" >&2
  exit 1
fi

if [ ! -x "$DAEMON_BINARY" ]; then
  echo "warning: $DAEMON_BINARY not found — run deploy/install.sh first;" >&2
  echo "the plugin installs fine but 'Test one frame' will fail until then" >&2
fi

if [ "$UNINSTALL" -eq 1 ]; then
  if [ "$DIRECT" -eq 1 ]; then
    if [ -d "$DEST" ]; then
      case "$DEST" in
        "$PLUGINS_DIR"/*) sudo rm -rf -- "$DEST" ;;
        *) echo "refusing to remove unexpected path: $DEST" >&2; exit 1 ;;
      esac
      echo "removed $DEST"
    else
      echo "$DEST does not exist, nothing to remove"
    fi
    if systemctl cat "$LOADER_UNIT" >/dev/null 2>&1; then
      sudo systemctl restart "$LOADER_UNIT"
      echo "restarted $LOADER_UNIT"
    fi
  else
    command -v node >/dev/null 2>&1 || { echo "node is required (plugin/decky-api.mjs)" >&2; exit 1; }
    node "$SCRIPT_DIR/decky-api.mjs" uninstall "$PLUGIN_NAME"
    echo "uninstalled «$PLUGIN_NAME» through the loader API"
  fi
  exit 0
fi

if [ "$DIRECT" -eq 1 ]; then
  # Direct copy: the plugins dir is root-owned under Decky Loader.
  if [ ! -d "$PLUGINS_DIR" ]; then
    echo "Decky Loader not found: $PLUGINS_DIR does not exist" >&2
    exit 1
  fi
  if [ -d "$DEST" ]; then
    case "$DEST" in
      "$PLUGINS_DIR"/*) sudo rm -rf -- "$DEST" ;;
      *) echo "refusing to touch unexpected path: $DEST" >&2; exit 1 ;;
    esac
  fi
  sudo install -d -m 0755 "$PLUGINS_DIR" "$DEST" "$DEST/dist"
  sudo install -m 0644 "$SCRIPT_DIR/plugin.json" "$SCRIPT_DIR/package.json" "$SCRIPT_DIR/main.py" "$DEST/"
  sudo install -m 0644 "$SCRIPT_DIR"/dist/* "$DEST/dist/"
  if systemctl cat "$LOADER_UNIT" >/dev/null 2>&1; then
    sudo systemctl restart "$LOADER_UNIT"
    echo "restarted $LOADER_UNIT — the plugin appears in the Decky menu"
    echo "logs: journalctl -u $LOADER_UNIT -f"
  else
    echo "warning: no $LOADER_UNIT unit found, not restarting" >&2
    echo "reload plugins from the Decky Loader settings in the Steam UI" >&2
  fi
  echo "installed $DEST (direct mode)"
  exit 0
fi

# Default: install through the loader's local API — no sudo, no restart.
command -v node >/dev/null 2>&1 || { echo "node >= 22 is required (plugin/decky-api.mjs)" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required to pack the zip" >&2; exit 1; }

ZIP="$(mktemp -t "$PLUGIN_DIR_NAME-XXXXXX.zip")"
trap 'rm -f "$ZIP"' EXIT
chmod 0644 "$ZIP"
python3 - "$SCRIPT_DIR" "$PLUGIN_DIR_NAME" "$ZIP" <<'PY'
import os, sys, zipfile
src, top, out = sys.argv[1], sys.argv[2], sys.argv[3]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for f in ("plugin.json", "package.json", "main.py"):
        z.write(os.path.join(src, f), f"{top}/{f}")
    dist = os.path.join(src, "dist")
    for f in sorted(os.listdir(dist)):
        z.write(os.path.join(dist, f), f"{top}/dist/{f}")
PY

node "$SCRIPT_DIR/decky-api.mjs" install "$ZIP" "$PLUGIN_NAME"
echo "installed «$PLUGIN_NAME» into $DEST through the loader API (no sudo)"
echo "it appears in the Decky quick-access menu; plugin logs: $DECKY_HOME/logs/$PLUGIN_DIR_NAME/"
echo "to reinstall after editing the plugin: ./plugin/install.sh --build"
