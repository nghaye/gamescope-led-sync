#!/usr/bin/env bash
# package-plugin.sh — build the distributable Decky plugin zip (SPEC §11).
#
#   scripts/package-plugin.sh
#
# Produces, in dist/:
#   gamescope-led-sync-decky-<version>.zip        # import this into Decky Loader
#   gamescope-led-sync-decky-<version>.zip.sha256
#
# The zip embeds the prebuilt payload the plugin installs on the Steam Deck —
# binary + systemd user unit + example config + calibration .desktop + license —
# under gamescope-led-sync/payload/, next to the plugin itself (plugin.json,
# package.json, main.py, dist/). The version is read from Cargo.toml and must
# match plugin/plugin.json and plugin/package.json (`just check-versions`).
#
# Steps: version check -> cargo build --release --locked -> strip ->
# npm ci + npm run build (plugin/) -> fill plugin/payload/ -> zip + sha256.
# Needs: cargo, npm, python3, sha256sum; strip is optional.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_DIR"

PLUGIN_DIR_NAME="gamescope-led-sync"
PLUGIN_DIR="$REPO_DIR/plugin"
DIST_DIR="$REPO_DIR/dist"
PAYLOAD="$PLUGIN_DIR/payload"

# --- version: Cargo.toml is the source of truth, all three must agree ---------
VERSION="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')"
v_plugin="$(python3 -c 'import json; print(json.load(open("plugin/plugin.json"))["version"])')"
v_pkg="$(python3 -c 'import json; print(json.load(open("plugin/package.json"))["version"])')"
if [ "$VERSION" != "$v_plugin" ] || [ "$VERSION" != "$v_pkg" ]; then
  echo "version mismatch: Cargo.toml=$VERSION plugin/plugin.json=$v_plugin plugin/package.json=$v_pkg" >&2
  echo "run: just bump-version $VERSION" >&2
  exit 1
fi
echo "== version $VERSION"

# --- build the release binary ---------------------------------------------------
echo "== cargo build --release --locked"
cargo build --release --locked
BIN="$REPO_DIR/target/release/$PLUGIN_DIR_NAME"
[ -f "$BIN" ] || { echo "binary not found after build: $BIN" >&2; exit 1; }
if command -v strip >/dev/null 2>&1; then
  # The zip ships the binary; the release profile already sets debug=false
  # and lto, strip only drops what is left.
  strip "$BIN"
fi

# --- build the plugin frontend ---------------------------------------------------
echo "== plugin frontend build"
(
  cd "$PLUGIN_DIR"
  if [ -f package-lock.json ]; then npm ci; else npm install; fi
  npm run build
)
[ -f "$PLUGIN_DIR/dist/index.js" ] || { echo "plugin/dist/index.js missing after build" >&2; exit 1; }

# --- fill the payload -------------------------------------------------------------
echo "== payload"
rm -rf "$PAYLOAD"
mkdir -p "$PAYLOAD/bin" "$PAYLOAD/systemd" "$PAYLOAD/config" "$PAYLOAD/desktop"
install -m 0755 "$BIN" "$PAYLOAD/bin/$PLUGIN_DIR_NAME"
install -m 0644 "$REPO_DIR/deploy/$PLUGIN_DIR_NAME.service" "$PAYLOAD/systemd/$PLUGIN_DIR_NAME.service"
install -m 0644 "$REPO_DIR/deploy/config.toml.example" "$PAYLOAD/config/config.toml.example"
install -m 0644 "$REPO_DIR/deploy/gamescope-led-sync-calibrate.desktop" "$PAYLOAD/desktop/gamescope-led-sync-calibrate.desktop"
install -m 0644 "$REPO_DIR/LICENSE" "$PAYLOAD/LICENSE"

# --- zip --------------------------------------------------------------------------
echo "== zip"
mkdir -p "$DIST_DIR"
ZIP="$DIST_DIR/${PLUGIN_DIR_NAME}-decky-$VERSION.zip"
rm -f "$ZIP" "$ZIP.sha256"
python3 - "$PLUGIN_DIR" "$PAYLOAD" "$PLUGIN_DIR_NAME" "$ZIP" <<'PY'
import os, sys, zipfile
plugin, payload, top, out = sys.argv[1:5]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for f in ("plugin.json", "package.json", "main.py"):
        z.write(os.path.join(plugin, f), f"{top}/{f}")
    dist = os.path.join(plugin, "dist")
    for f in sorted(os.listdir(dist)):
        z.write(os.path.join(dist, f), f"{top}/dist/{f}")
    for root, _dirs, files in os.walk(payload):
        for f in sorted(files):
            full = os.path.join(root, f)
            z.write(full, f"{top}/payload/{os.path.relpath(full, payload)}")
PY
(cd "$DIST_DIR" && sha256sum "$(basename "$ZIP")" > "$(basename "$ZIP").sha256")

# --- recap ------------------------------------------------------------------------
echo "== done"
ls -l "$ZIP" "$ZIP.sha256"
(cd "$DIST_DIR" && sha256sum -c "$(basename "$ZIP").sha256")
python3 - "$ZIP" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    for n in z.namelist():
        print(f"  {n}")
PY
echo "import this zip into Decky Loader, then open the plugin and press «Installer»"
