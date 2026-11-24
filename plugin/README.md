# Gamescope LED Sync — Decky plugin

Decky Loader plugin for [gamescope-led-sync](https://github.com/nghaye/gamescope-led-sync):
live configuration of the ambient LED backlight daemon that streams gamescope's
capture to a WLED controller (SPEC §10). The plugin runs as the session user
(_no root flag_, D6) and the UI is in English (D7). It has two pages, and the
plugin is also the **install entry point** for the service + binary (§10.3,
see _Install into Decky Loader, then install the service from the plugin_
below).

## Main page

- **Installation** (polled every 5 s): plugin version, installed binary
  version, version check (a mismatch greys out every other control), and the
  Install / Start / Stop / Update / Uninstall buttons plus the «Start at
  boot» switch — see the install section below.
- **Status** (polled every 5 s): service state, capture state (capturing /
  waiting for gamescope / paused / inactive), live FPS, and WLED strip
  reachability (HTTP probe of the configured host, `GET /json/info`).
- **Capture**: the «Capture» switch pauses/resumes the capture live
  (SIGUSR1 in the daemon — while paused, nothing is sent and the WLED is
  released). The «Capture at startup» switch persists
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
`--preview-strip` mode, spawned by the backend with the current layout. When
the start point is an **edge midpoint**, that edge is split at the loop's
seam: the beginning of the strip keeps the segment color, the end is
**violet**, so you can see the seam and the circular offset on the strip.
Every change (segment lengths, start point, direction, offset, gap, WLED host)
**immediately respawns the preview** with the draft layout, so you can check
the segment lengths on the strip itself. **Nothing is written to the config
file until «Save and quit»** — the immediate effect goes exclusively through
the preview; «Cancel» leaves the screen without saving.

Fields: WLED host/IP with a green **OK** / red **NOK** reachability indicator
(probed on change), start point (8 full-word options, no codes), clockwise
rotation switch (off = counterclockwise), the four segment LED counts each
flanked by **−1 / +1** buttons (stepping within the 0..=490 / count ≤ 490
bounds, so no keyboard is needed) with a live **total active LEDs** line,
the offset slider (−20 … +20, step 1 — a
circular shift of the whole strip, the last LED being next to the first), the
gap slider (0 … 50, step 1 — a hole at the loop seam between the last and
first LED, e.g. a cable duct; the creux region is skipped when sampling the
screen and the packet length stays the active LED count), the
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

From the repository root, `just package-plugin` does all of it and more: it
also builds the release binary (`cargo build --release --locked` + `strip`),
fills `plugin/payload/` (gitignored) with `bin/gamescope-led-sync`,
`systemd/gamescope-led-sync.service`, `config/config.toml.example` and
`LICENSE`, and packs the **distributable zip**
`dist/gamescope-led-sync-decky-<version>.zip` (+ `.sha256`), with the
`gamescope-led-sync/` root folder the loader expects.

## Install into Decky Loader, then install the service from the plugin

1. Import the zip (`dist/gamescope-led-sync-decky-<version>.zip`) in the Steam
   UI: **Decky Loader → Settings → install a plugin zip**. The loader unzips
   it into `~/homebrew/plugins/gamescope-led-sync` — the embedded `payload/`
   comes along.
2. Open the plugin and press **Install service + binary** in the
   **Installation** section (works in Big Picture with a controller): the
   backend copies the payload's binary to `~/.local/bin/gamescope-led-sync`
   (atomically, 0755), the unit to `~/.config/systemd/user/`, the example
   config to `~/.config/gamescope-led-sync/config.toml` **only if absent**,
   then `systemctl --user daemon-reload` + `enable --now`. No sudo (the plugin
   runs as the session user).

The Installation section also shows the plugin version and the installed
binary's version (`--version`); when they differ, only **Update** and
**Uninstall** stay enabled (every other control is greyed out).
**Update** stops the service if running, backs the old binary up to `.bak`,
replaces binary + unit, `daemon-reload`, and restarts as before (rollback from
the `.bak` on failure). **Uninstall** (with a confirmation dialog) removes the
unit and the binary and **keeps the config file**.

From a checkout, `just install-all` does steps 1–2 automatically: it packages
the zip, installs the plugin through the loader's own local API
(`plugin/decky-api.mjs`: `GET 127.0.0.1:1337/auth/token` is unauthenticated on
localhost, then `utilities/install_plugin` + `confirm_plugin_install` over
`/ws` with the zip as a `file://` artifact — no sudo, no loader restart), and
installs the service + binary from the fresh release build
(`scripts/install-service.sh --enable`). `just install-all --uninstall`
removes both.

Note: the served URLs use the plugin.json display name («Gamescope LED
Sync», URL-encoded), not the install folder — see `src/index.tsx`.
