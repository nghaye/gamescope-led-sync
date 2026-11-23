# gamescope-led-sync

Ambient LED backlight for [gamescope](https://github.com/ValveSoftware/gamescope) sessions, as a SteamOS deliverable.

Built for a DIY Steam machine on CachyOS: Steam runs in a **gamescope** session (SDDM with autologin, BC250 motherboard), and an addressable LED strip runs around a **65" TV** (perimeter ≈ 4.5 m, ≈ 60 LEDs/m → **≈ 268 LEDs**), driven by a **WLED** controller on the same LAN.

The HDMI signal is never touched (no splitter, no sync box): the program captures gamescope's own PipeWire stream, computes one color per LED, and sends them to WLED as DRGB realtime UDP packets.

## How it works

The pipeline has four steps:

1. gamescope draws each frame and publishes it on PipeWire.
2. gamescope-led-sync reads the stream, shrinks it, and computes one color for each LED around the screen edges.
3. It sends the colors to the WLED controller over the local network, as DRGB realtime UDP packets.
4. WLED drives the addressable LED strip around the TV.

One small Rust binary links libpipewire and runs on Linux, with near-zero cost to the game: it asks gamescope to shrink the capture on the GPU (custom `requested_size` negotiation), and it is fully local — no account, no cloud, no telemetry.

## Requirements

- Linux with a PipeWire **user session** (SteamOS, Bazzite, ChimeraOS, CachyOS, or a custom build).
- `systemd --user` (SDDM autologin provides one).
- A WLED controller on the same LAN, UDP realtime enabled, protocol **DRGB**, port 21324 (or as configured), hostname resolvable (`wled.local` via mDNS, or an IP).
- **At most 490 LEDs** (one DRGB packet; see _Protocol notes_).

## Install

Clone [this repository](https://github.com/nghaye/gamescope-led-sync), then:

```sh
cd gamescope-led-sync
./deploy/install.sh --enable
```

`install.sh` is idempotent: it builds with `cargo build --release --locked` (or takes `--binary PATH`), installs the binary to `/usr/bin/gamescope-led-sync`, copies `deploy/config.toml.example` to `~/.config/gamescope-led-sync/config.toml` **only if absent**, installs the systemd user unit to `~/.config/systemd/user/gamescope-led-sync.service`, and with `--enable` runs `systemctl --user enable --now gamescope-led-sync`.

Uninstall with `./deploy/install.sh --uninstall`: it disables and stops the unit, removes the unit and the binary, and **keeps the config file**.

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

One flag per config field, CLI-only flags aside: `--config PATH`, `--setup` (interactive setup, writes the config file and exits), `--dry-run` (never send, log only), `--wled HOST`, `--port`, `--timeout`, `--top/--right/--bottom/--left`, `--start <8 codes>`, `--direction <clockwise|counterclockwise>`, `--offset N`, `--gap N`, `--fps`, `--sample-height`, `--band`, `--saturation`, `--gamma`, `--smooth`, `--brightness`, `--start-paused` (start with the capture paused), plus debug-only `--once`, `--dump PNG`, `--preview`, `--preview-strip` (send the per-segment test pattern to WLED until SIGTERM). Validation errors exit with a clap-style error (exit code 2); an explicit `--config` pointing at a missing or unparseable file is fatal at startup (exit code 2). Run `gamescope-led-sync --help` for the full list.

## Service management

```sh
systemctl --user enable --now gamescope-led-sync   # start at boot + now
systemctl --user stop gamescope-led-sync
systemctl --user status gamescope-led-sync
journalctl --user -u gamescope-led-sync -f         # logs
```

The daemon waits for the gamescope node (reconnect loop), so starting it at boot before Steam is fine. The [Decky plugin](./plugin) configures everything live from the Steam UI.

## Strip geometry

The perimeter is a cyclic list of `count = top+right+bottom+left` zones. The strip's first LED sits at one of **8 start points** (4 corners `bl tl tr br` + 4 edge midpoints `lc tc rc bc`) and runs in the chosen **direction** (`clockwise`/`counterclockwise`, viewed from the front) — 16 combinations.

**Offset**: the strip is a **loop** — the last LED is physically next to the first — so `offset` is a circular shift of the whole strip: LED `i` gets zone `chain[(i − offset) mod count]`. `offset > 0` shifts the image toward higher LED indices (right along the strip), `offset < 0` toward lower indices (left), wrapping around the loop. The packet length is always `count` (≤ 490), whatever the offset — so changing it, sign included, can never break the WLED stream.

**Gap (seam hole)**: `gap` (0..=50, default 0) is a hole at the loop seam, in LED pitches, between the strip's **last and first LED** — e.g. a cable duct at the bottom center when the strip starts at `bc`. The seam edge is then divided into `n+gap` zones and the `gap` zones at the seam (the creux) are skipped, so no LED shows the screen region behind the creux and each LED shows the zone physically in front of it. The packet length stays `count`: the hole has no LEDs, and wrapping the offset across the seam jumps it.

**WLED segment-length rule**: the packet always starts at WLED LED index 0 and always carries the whole chain, so set your WLED segment length to `count` (e.g. 268 for the 65" strip). The offset only shifts the colors circularly within that segment; the gap does not change the segment length.

## Decky plugin

`plugin/` is a [Decky Loader](https://github.com/SteamDeckHomebrew/decky-loader) plugin (no root flag; runs as the session user) with two pages:

- **Main page**: status lines (service, capture, live FPS, WLED strip reachability — polled every 5 s), a **Capture** switch (SIGUSR1 pause/resume; while paused nothing is sent and the WLED is released), a **Capture at startup** switch (`[capture] start_paused`, persisted in the config file and applied immediately on change), the **Brightness / Gamma / Saturation / Smooth** sliders, and a **Configuration** button. Sliders and switches save on change (debounced 400 ms) and go live at the daemon's next tick, and are reloaded at boot.
- **Configuration screen**: entering it pauses the capture (resumed on exit) and lights each of the 4 strip segments in its own color (bottom red, left blue, top green, right yellow) via the daemon's `--preview-strip` mode; when the start point is an edge midpoint, that edge is split at the loop's seam — beginning keeps the segment color, end is violet — so the circular offset is visible on the strip. Every change is previewed live on the strip but **written to the config file only on «Save and quit»** («Cancel» leaves without saving). Fields: WLED host/IP with a green OK / red NOK reachability check, start point (8 full-word options), clockwise rotation switch, per-segment LED counts each flanked by −1/+1 buttons, with a live total, offset slider (−20…+20, circular shift of the loop), gap slider (0…50, hole at the loop seam between the last and first LED — the creux is skipped when sampling the screen), screen band slider (1–20 %), FPS.

`wled.timeout_s`, `wled.port`, `sample_height`, `behavior.on_no_session` and `capture.node_name` stay file-only. `deploy/install.sh` does **not** install the plugin — run `./plugin/install.sh` once as the normal user (no sudo: it installs through Decky Loader's local API; `--direct` falls back to a sudo copy + loader restart). See [plugin/README.md](./plugin/README.md).

## Protocol notes

WLED UDP realtime, protocol byte 2 (**DRGB**): a timeout byte (seconds WLED stays in realtime after the last packet), then three bytes per LED. One packet carries at most **490 LEDs**; beyond that, WLED's DNRGB multi-packet protocol would be needed — out of scope for v1.

While no gamescope session is connected, `on_no_session` decides what is sent: `preset` (default) sends nothing and WLED falls back to its own preset after `timeout_s`; `keep-last` resends the last colors every 1 s; `black` sends black every 1 s. During an active session a 1 s keepalive resends the last colors on static screens, so WLED never drops to its preset mid-game. A manual pause (the Decky «Capture» switch off, or `start_paused = true`) sends nothing at all — whatever `on_no_session` says — so WLED's realtime timeout releases the strip.

## Troubleshooting

- Logs: `journalctl --user -u gamescope-led-sync -f`.
- Capture one frame and save it: `gamescope-led-sync --once --dump frame.png`.
- See the edge colors live on a terminal: `--preview`.
- Never send anything, log only: `--dry-run`.
- Use a custom config file: `--config /path/to/config.toml`.

## Development

```sh
just ci        # fmt-check, clippy -D warnings, cargo test, nix/toml/yaml/markdown lints,
               # plus the plugin: biome format/lint, tsc, rollup build, py_compile of main.py
cargo test
cargo run -- --help
```

The unit tests cover the geometry truth table (16 start × direction combos), offset cases, color grading, config parse/validate, CLI > file > default precedence, and live reload; `tests/cli.rs` covers the exit-code-2 contract and the `--help` surface at the process level.

Manual E2E checklist: `./deploy/install.sh --enable`; launch a game and verify the LEDs follow the screen edges; in the Decky plugin's configuration screen, change a segment length and see the colored preview update immediately, then «Save and quit» and watch the live capture resume with the new layout; try `--once --dump` + `--preview`; try `--dry-run`.

## License

Copyright 2026 larry0x, nghaye.

Licensed under the [Apache License, Version 2.0](./LICENSE).
