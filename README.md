# gamescope-led-sync

Ambient LED backlight for [gamescope](https://github.com/ValveSoftware/gamescope) sessions: the HDMI signal is never touched (no splitter, no sync box) — the program captures gamescope's own PipeWire stream, computes one color per LED around the screen, and sends them to a **WLED** controller on the same LAN as DRGB realtime UDP packets.

Tested on **SteamOS** (CachyOS, gamescope session started from SDDM with autologin) with a ~268-LED strip around a 65" TV driven by WLED. Provided **as-is**, without warranty — use at your own risk.

## How it works

The pipeline has four steps:

1. gamescope draws each frame and publishes it on PipeWire.
2. gamescope-led-sync reads the stream, shrinks it, and computes one color for each LED around the screen edges.
3. It sends the colors to the WLED controller over the local network.
4. WLED drives the addressable LED strip around the TV.

One small Rust binary links libpipewire and runs on Linux, with near-zero cost to the game: it asks gamescope to shrink the capture on the GPU (custom `requested_size` negotiation), and it is fully local — no account, no cloud, no telemetry.

## Requirements

- Linux with a PipeWire **user session** (SteamOS, Bazzite, ChimeraOS, CachyOS, or a custom build).
- `systemd --user` (SDDM autologin provides one).
- A WLED controller on the same LAN, UDP realtime enabled, protocol **DRGB**, port 21324 (or as configured), hostname resolvable (`wled.local` via mDNS, or an IP).
- **At most 490 LEDs** (they must fit in a single DRGB UDP packet).

## Limitations

- **gamescope sessions only**: the capture reads the PipeWire node whose `media.name` is `gamescope` (`[capture] node_name` is the escape hatch if yours is named differently). On a regular desktop session there is no such node, so nothing is captured and the strip follows `on_no_session` (default: WLED falls back to its own preset after `timeout_s`).
- **One strip, one WLED controller** (and at most 490 LEDs — see _Requirements_).
- **Brightness is capped by WLED**: the daemon's `brightness` can only scale colors down, so the effective maximum is the LED brightness configured in WLED itself (plus its current limit). Raise it in WLED if the strip is too dim.
- **A single hole, at the seam**: `gap` can only skip the region between the last and first LED; holes elsewhere along the strip are not supported.
- **Linux + PipeWire + `systemd --user` only** — no other platform, no autostart without a user systemd session.
- In the Decky UI the offset slider is limited to −20…+20; the config file accepts any integer.

## Install

The main path is the **Decky plugin zip**, which embeds the prebuilt binary, the
systemd user unit, an example config and the license:

1. Get `gamescope-led-sync-decky-<version>.zip` (built by `just package-plugin`
   from a checkout of [this repository](https://github.com/nghaye/gamescope-led-sync),
   into `dist/`, with a `.sha256` next to it).
2. In Steam, open **Decky Loader → Settings → install a plugin zip** and import
   the zip.
3. Open the **Gamescope LED Sync** plugin and press **Install service + binary**
   (Big Picture, controller only): the plugin copies the binary to
   `~/.local/bin/gamescope-led-sync`, the unit to
   `~/.config/systemd/user/gamescope-led-sync.service`, the example config to
   `~/.config/gamescope-led-sync/config.toml` (only if absent), the calibration
   launcher to `~/.local/share/applications/gamescope-led-sync-calibrate.desktop`
   (see _LED color calibration_ below), then `daemon-reload` + `enable --now`.
   No sudo — everything is user-space.

The plugin's **Installation** section shows the plugin version, the installed
version, and whether they match; on a mismatch (new zip imported, old service
installed — or the reverse) only **Update** and **Uninstall** stay enabled.
**Update** stops the service if running, replaces binary + unit (with a `.bak`
backup and rollback on failure), and restarts it as before. **Uninstall**
(after a confirmation) removes the unit and the binary and **keeps the config
file**.

For development / headless installs from a clone:

```sh
just install-all        # build + install the plugin (loader local API, no sudo) + service + binary
just package-plugin     # build only the distributable zip into dist/
scripts/install-service.sh --uninstall   # remove service + binary (config kept)
```

## Configuration

Configuration lives in a TOML file; lookup order: `--config PATH` if given, else `$XDG_CONFIG_HOME/gamescope-led-sync/config.toml` (`~/.config/...`), else `/etc/gamescope-led-sync/config.toml`, else built-in defaults.

First run? `gamescope-led-sync --setup` asks interactively for the WLED host and the strip layout (LED count per edge, start point, direction), tests the connection to the controller and lights each edge in its own color so you can check the counts, lets you go back and fix any setting, and writes the config file; every other parameter is left at its default.

Value precedence: **CLI flag > config file > built-in default**. The daemon stats the file every tick and applies a valid change live at the next tick (≤ 1/fps ≈ 33 ms at 30 fps); an invalid or deleted file keeps the previous config active. Unknown keys are a hard error, so a typo cannot pass silently.

| Section       | Key             | Default       | Bound / codes                          |
| ------------- | --------------- | ------------- | -------------------------------------- |
| `[wled]`      | `host`          | _none_        | non-empty hostname or IP               |
|               | `port`          | `21324`       | `1..=65535`                            |
|               | `timeout_s`     | `2`           | `1..=255`, sent in every packet header |
| `[layout]`    | `top/right/bottom/left` | `132/75/132/75` | `0..=490` each, `count ≥ 1`      |
|               | `start`         | `"bl"`        | `bl bc br rc tr tc tl lc`              |
|               | `direction`     | `"clockwise"` | `clockwise\|counterclockwise`          |
|               | `offset`        | `0`           | integer; circular shift, packet length stays `count ≤ 490` |
|               | `gap`           | `0`           | `0..=50`; hole at the loop seam (last↔first LED) in LED pitches, skipped when sampling the screen; packet length stays `count` |
| `[processing]`| `fps`           | `30`          | `1..=240`                              |
|               | `sample_height` | `240`         | `1..=16384`                            |
|               | `band_frac`     | `0.08`        | `> 0 and < 1`                          |
|               | `saturation`    | `1.0`         | `≥ 0`                                  |
|               | `gamma`         | `1.0`         | `> 0`                                  |
|               | `smooth`        | `0.5`         | `> 0`; `1.0` disables smoothing        |
|               | `brightness`    | `1.0`         | `0.0..=1.0`                            |
| `[behavior]`  | `on_no_session` | `"preset"`    | `keep-last\|black\|preset`             |
| `[capture]`   | `node_name`     | `"gamescope"` | non-empty `media.name` of the node     |
|               | `start_paused`  | `false`       | bool; start with the capture paused (Decky «Capture at startup») |

See [deploy/config.toml.example](./deploy/config.toml.example) for a fully commented example (the 65" strip: `top=86 right=48 bottom=86 left=48`, `start="bc"`, `direction="clockwise"`, `offset=0`, `gap=0`).

### CLI overrides

One flag per config field, CLI-only flags aside: `--config PATH`, `--setup` (interactive setup, writes the config file and exits), `--dry-run` (never send, log only), `--wled HOST`, `--port`, `--timeout`, `--top/--right/--bottom/--left`, `--start <8 codes>`, `--direction <clockwise|counterclockwise>`, `--offset N`, `--gap N`, `--fps`, `--sample-height`, `--band`, `--saturation`, `--gamma`, `--smooth`, `--brightness`, `--start-paused` (start with the capture paused), plus debug-only `--once`, `--dump PNG`, `--preview`, `--preview-strip` (send the per-segment test pattern to WLED until SIGTERM), and `--calibrate` (stand-alone full-screen LED color calibration, see below). Validation errors exit with a clap-style error (exit code 2); an explicit `--config` pointing at a missing or unparseable file is fatal at startup (exit code 2). Run `gamescope-led-sync --help` for the full list.

## Service management

```sh
systemctl --user enable --now gamescope-led-sync   # start at boot + now
systemctl --user stop gamescope-led-sync
systemctl --user status gamescope-led-sync
journalctl --user -u gamescope-led-sync -f         # logs
```

The daemon waits for the gamescope node (reconnect loop), so starting it at boot before Steam is fine. The [Decky plugin](./plugin) configures everything live from the Steam UI.

## LED color calibration

If the strip's colors drift from the screen (e.g. a yellow leaning orange on the LEDs), run the stand-alone calibration. It is installed as `~/.local/share/applications/gamescope-led-sync-calibrate.desktop` — in Steam, add it as a **non-Steam game** ("Calibration LED (gamescope-led-sync)") and launch it from Big Picture. It runs full-screen, 100% controller-driven:

- On start it pauses the daemon's capture (SIGUSR1) so the strip is yours alone, and resumes it on exit.
- Each of 24 test colors is shown full-screen while the whole strip lights in the slider-adjusted color at **60%** brightness. Tune the **R/G/B sliders** until the strip matches the screen, then go to the next color. Every page has **Previous / Cancel / Next** buttons. Controls: D-pad/stick = navigate, left/right = adjust (auto-repeat), **A** = confirm, **B** = cancel, **LB/RB** = previous/next (keyboard fallback: arrows, Enter, Esc, PgUp/PgDn).
- After the last color, a report page shows the fitted per-channel gains and the **WLED white balance** values to enter — WLED web UI: _Settings → LED preferences → "Color white"_: **Red / Green / Blue** (0–255, 255 = no correction) — plus warnings (e.g. suspected Color Order permutation). The full report is saved to `~/.config/gamescope-led-sync/calibration-<UTC timestamp>.txt`. **Quit** releases the WLED realtime mode immediately and resumes the capture.

From a shell it is the same: `gamescope-led-sync --calibrate` (`--dry-run` logs the strip colors instead of sending them). Details in [SPEC.md §13](./SPEC.md).

## Strip geometry

The perimeter is a cyclic list of `count = top+right+bottom+left` zones. The strip's first LED sits at one of **8 start points** (4 corners `bl tl tr br` + 4 edge midpoints `lc tc rc bc`) and runs in the chosen **direction** (`clockwise`/`counterclockwise`, viewed from the front) — 16 combinations.

**Offset**: the strip is a **loop** — the last LED is physically next to the first — so `offset` is a circular shift of the whole strip: LED `i` gets zone `chain[(i − offset) mod count]`. `offset > 0` shifts the image toward higher LED indices (right along the strip), `offset < 0` toward lower indices (left), wrapping around the loop. The packet length is always `count` (≤ 490), whatever the offset — so changing it, sign included, can never break the WLED stream.

**Gap (seam hole)**: `gap` (0..=50, default 0) is a hole at the loop seam, in LED pitches, between the strip's **last and first LED** — e.g. a cable duct at the bottom center when the strip starts at `bc`. The seam edge is then divided into `n+gap` zones and the `gap` zones at the seam (the creux) are skipped, so no LED shows the screen region behind the creux and each LED shows the zone physically in front of it. The packet length stays `count`: the hole has no LEDs, and wrapping the offset across the seam jumps it.

**WLED segment-length rule**: the packet always starts at WLED LED index 0 and always carries the whole chain, so set your WLED segment length to `count` (e.g. 268 for the 65" strip). The offset only shifts the colors circularly within that segment; the gap does not change the segment length.

## Decky plugin

A [Decky Loader](https://github.com/SteamDeckHomebrew/decky-loader) plugin — no root, it runs as the session user — to install the service and configure everything live from the Steam UI, fully gamepad-navigable. Import its zip into Decky Loader (see _Install_ above); from a checkout, `just install-all` does everything. See [plugin/README.md](./plugin/README.md).

- **Installation**: install / update / uninstall the binary + systemd unit from the zip's embedded payload (no sudo, user-space only), version check against the installed binary, «Start at boot» switch.
- **Status** (polled every 5 s): service state, capture state, live FPS, WLED reachability.
- **Capture**: pause/resume switch (while paused, nothing is sent to WLED and it is released) and «Capture at startup» switch (`[capture] start_paused`).
- **Image**: brightness, gamma, saturation, smooth sliders — saved on change (debounced), live at the daemon's next tick.
- **Configuration screen**: WLED host with reachability check, start point, direction, per-segment LED counts, offset, gap, screen band, FPS — every change previewed live on the strip (one color per segment, `--preview-strip`), written to the config only on «Save and quit».

## Troubleshooting

- Logs: `journalctl --user -u gamescope-led-sync -f`.
- Capture one frame and save it: `gamescope-led-sync --once --dump frame.png`.
- See the edge colors live on a terminal: `--preview`.
- Never send anything, log only: `--dry-run`.
- Use a custom config file: `--config /path/to/config.toml`.

## Development

```sh
just ci             # check-versions + fmt-check, clippy -D warnings, cargo test, nix/toml/yaml/markdown lints,
                    # plus the plugin: biome format/lint, tsc, rollup build, py_compile + unit tests of main.py
just package-plugin # build the distributable Decky zip (with embedded payload) into dist/
just install-all    # build + install plugin (loader API) + service + binary from this checkout
just bump-version 0.2.0   # set the shared version in Cargo.toml + plugin.json + package.json
cargo test
cargo run -- --help
```

The unit tests cover the geometry truth table (16 start × direction combos), offset cases, color grading, config parse/validate, CLI > file > default precedence, and live reload; `tests/cli.rs` covers the exit-code-2 contract, the `--version` line and the `--help` surface at the process level; `plugin/tests/` covers the install/update/uninstall logic of the plugin backend with a fake systemctl.

Manual E2E checklist: `just install-all`; launch a game and verify the LEDs follow the screen edges; in the Decky plugin's configuration screen, change a segment length and see the colored preview update immediately, then «Save and quit» and watch the live capture resume with the new layout; try `--once --dump` + `--preview`; try `--dry-run`; in the plugin's Installation section, press Stop / Start, then re-import a rebuilt zip and press Update.

## License

Copyright 2026 larry0x, nghaye.

Licensed under the [Apache License, Version 2.0](./LICENSE).
