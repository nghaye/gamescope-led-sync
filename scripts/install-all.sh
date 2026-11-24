#!/usr/bin/env bash
# install-all.sh — developer entry point: build and install everything from
# this checkout (SPEC §11).
#
#   scripts/install-all.sh
#   scripts/install-all.sh --uninstall
#
# Install: package the distributable zip first (scripts/package-plugin.sh:
# release binary + plugin frontend + embedded payload + sha256), then install
# the plugin into Decky Loader through the loader's own local API
# (plugin/decky-api.mjs — GET 127.0.0.1:1337/auth/token is unauthenticated on
# localhost, utilities/install_plugin over /ws takes the zip as file:// — no
# sudo, no loader restart), then install the service + binary as the user from
# the freshly built release binary (scripts/install-service.sh --binary …
# --enable: ~/.local/bin + ~/.config/systemd/user + daemon-reload + enable
# --now).
#
# --uninstall: remove the plugin through the loader API, then
# install-service.sh --uninstall (the config file is kept).
#
# Needs: cargo, npm, python3, node >= 22, sha256sum; Decky Loader running.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_DIR"

PLUGIN_DIR_NAME="gamescope-led-sync"
PLUGIN_NAME="$(python3 -c 'import json; print(json.load(open("plugin/plugin.json"))["name"])')"
VERSION="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')"
ZIP="$REPO_DIR/dist/${PLUGIN_DIR_NAME}-decky-$VERSION.zip"

UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help)
      sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

command -v node >/dev/null 2>&1 || { echo "node >= 22 is required (plugin/decky-api.mjs)" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 1; }

if [ "$UNINSTALL" -eq 1 ]; then
  node plugin/decky-api.mjs uninstall "$PLUGIN_NAME"
  echo "uninstalled «$PLUGIN_NAME» through the loader API"
  bash scripts/install-service.sh --uninstall
  exit 0
fi

# 1. Package: release binary + frontend + payload -> dist/…-decky-<version>.zip
bash scripts/package-plugin.sh

# 2. Install the plugin into Decky Loader through its local API (no sudo).
node plugin/decky-api.mjs install "$ZIP" "$PLUGIN_NAME"
echo "installed «$PLUGIN_NAME» into ~/homebrew/plugins/$PLUGIN_DIR_NAME through the loader API"
echo "plugin logs: ~/homebrew/logs/$PLUGIN_DIR_NAME/"

# 3. Install the service + binary from this checkout's release binary.
bash scripts/install-service.sh --binary "$REPO_DIR/target/release/$PLUGIN_DIR_NAME" --enable

echo "done — in the Decky menu, the plugin's Installation section shows the installed version"
echo "to update later from a checkout: just install-all   (or import the new zip in Decky Loader, then «Update» in the plugin)"
