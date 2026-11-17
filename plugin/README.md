# Gamescope LED Sync — Decky plugin

Decky Loader plugin for [gamescope-led-sync](https://github.com/nghaye/gamescope-led-sync):
live configuration of the ambient LED backlight daemon that streams gamescope's
capture to a WLED controller (SPEC §10). The plugin runs as the session user
(_no root flag_, D6) and the UI is in English (D7).

## What it does

- **Service**: enable at boot, status, Start / Stop / Restart, View logs
  (`journalctl --user`), and **Test one frame**, which runs the binary with
  `--once --dump` and shows the captured PNG (served from the plugin data dir
  at `/plugins/gamescope-led-sync/data/last-frame.png`). Test one frame is
  refused while the service is active: two capture clients on one gamescope
  node are unsupported.
- **WLED**: host and UDP realtime port.
- **LED strip**: the four edge counts, the start point (8 options), the
  direction, and the offset slider (−30 … +30, step 1). The hint line shows
  the packet length `L = count + max(offset, 0)` and is highlighted whenever
  L changes — set your WLED segment length to L, the packet always starts at
  WLED LED index 0.
- **Image**: saturation, gamma, brightness, band, smooth sliders, numeric fps,
  and an advanced numeric sample height.

Every control saves on change, debounced 400 ms, through `save_config`: the
backend validates the config (Python mirror of SPEC §4.3) and writes the file
**atomically** (temp file + rename); the daemon applies it at the next tick
(≤ 1/fps), no restart and no Apply button. The status line shows _Saved ✓_ or
the validation errors, and the file is not written when invalid.

## File-only settings

`wled.timeout_s` and `behavior.on_no_session` are **not** in the UI (D9/D5):
edit `~/.config/gamescope-led-sync/config.toml` directly.

## Build

```sh
cd plugin
npm ci            # or `npm install` when there is no package-lock.json
npm run build     # rollup -> dist/index.js
```

## Install into Decky Loader

`deploy/install.sh` installs only the daemon (binary + systemd unit + config)
— the plugin is installed separately, **as the normal user, no sudo**:

```sh
./plugin/install.sh              # builds if dist/index.js is missing, then installs
./plugin/install.sh --build      # force a rebuild first
./plugin/install.sh --uninstall  # remove the plugin (data and settings kept)
./plugin/install.sh --direct     # fallback: sudo copy + restart plugin_loader
```

By default the script packs `plugin.json`, `package.json`, `main.py` and
`dist/` into a zip and calls Decky Loader's own local API
(`plugin/decky-api.mjs`: `GET 127.0.0.1:1337/auth/token` is unauthenticated on
localhost, then `utilities/install_plugin` + `confirm_plugin_install` over
`/ws` with the zip as a `file://` artifact). The loader — which runs as root —
unzips into `~/homebrew/plugins/gamescope-led-sync`, fixes ownership and
imports the plugin itself: no sudo, no loader restart, no Steam restart.
`--direct` instead copies the files with `sudo` (the plugins dir is
root-owned) and restarts the `plugin_loader` system service. The script is
idempotent: run it again after editing the plugin to reinstall. Override the
Decky home with `DECKY_HOME=...`.

Note: the served URLs use the plugin.json display name («Gamescope LED
Sync», URL-encoded), not the install folder — see `src/index.tsx`.

Manual alternative: build as above, then in the Steam UI open Decky Loader →
developer mode → install a zip of this folder.
