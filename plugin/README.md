# Gamescope LED Sync — Decky plugin

Decky Loader plugin for [gamescope-led-sync](https://github.com/nghaye/gamescope-led-sync):
live configuration of the ambient LED backlight daemon that streams gamescope's
capture to a WLED controller (SPEC §10). The plugin runs as the session user
(_no root flag_, D6) and the UI is in English (D7). It has two pages.

## Main page

- **Status** (polled every 5 s): service state, capture state (capturing /
  waiting for gamescope / paused / inactive), live FPS, and WLED strip
  reachability (HTTP probe of the configured host, `GET /json/info`).
- **Capture**: the «Capture» switch pauses/resumes the capture live
  (SIGUSR1 in the daemon — while paused, the configured no-session behavior
  drives the strip). The «Capture at startup» switch persists
  `[capture] start_paused` in the config file: the daemon starts paused when
  it is set, and toggling the switch also applies immediately on live reload.
- **Image**: Brightness, Gamma, Saturation, Smooth sliders.
- **Configuration**: opens the configuration screen.

Every slider and switch on the main page saves on change, debounced 400 ms,
through `save_config`: the backend validates the config (Python mirror of
SPEC §4.3) and writes the file **atomically** (temp file + rename); the
daemon applies it at the next tick (≤ 1/fps), no restart, and the value is
reloaded at boot.

## Configuration screen

Entering the screen **pauses the capture** (resumed on exit if it was
running) and lights each of the 4 strip segments in its own color — **bottom
red, left blue, top green, right yellow** — using the daemon's
`--preview-strip` mode, spawned by the backend with the current layout.
Every change (segment lengths, start point, direction, offset, WLED host)
**immediately respawns the preview** with the draft layout, so you can check
the segment lengths on the strip itself. **Nothing is written to the config
file until «Save and quit»** — the immediate effect goes exclusively through
the preview; «Cancel» leaves the screen without saving.

Fields: WLED host/IP with a green **OK** / red **NOK** reachability indicator
(probed on change), start point (8 full-word options, no codes), clockwise
rotation switch (off = counterclockwise), the four segment LED counts with a
live **total active LEDs** line, the offset slider (−20 … +20, step 1), the
screen band slider (1–20 %), and FPS.

«Save and quit» validates and writes the draft through `save_config`
(validation errors are shown and the screen stays open), stops the preview,
resumes the capture if it was paused on entry, and returns to the main page.

## File-only settings

`wled.timeout_s`, `wled.port`, `processing.sample_height`,
`behavior.on_no_session`, and `capture.node_name` are **not** in the UI:
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
