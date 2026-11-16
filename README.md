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

Value precedence: **CLI flag > config file > built-in default**. The daemon stats the file every tick and applies a valid change live at the next tick (≤ 1/fps ≈ 33 ms at 30 fps); an invalid or deleted file keeps the previous config active. Unknown keys are a hard error, so a typo cannot pass silently.

| Section       | Key             | Default       | Bound / codes                          |
| ------------- | --------------- | ------------- | -------------------------------------- |
| `[wled]`      | `host`          | _none_        | non-empty hostname or IP               |
|               | `port`          | `21324`       | `1..=65535`                            |
|               | `timeout_s`     | `2`           | `1..=255`, sent in every packet header |
| `[layout]`    | `top/right/bottom/left` | `132/75/132/75` | `0..=490` each, `count ≥ 1`      |
|               | `start`         | `"bl"`        | `bl bc br rc tr tc tl lc`              |
|               | `direction`     | `"clockwise"` | `clockwise\|counterclockwise`          |
|               | `offset`        | `0`           | packet length `count + max(offset,0) ≤ 490` |
| `[processing]`| `fps`           | `30`          | `1..=240`                              |
|               | `sample_height` | `240`         | `1..=16384`                            |
|               | `band_frac`     | `0.08`        | `> 0 and < 1`                          |
|               | `saturation`    | `1.0`         | `≥ 0`                                  |
|               | `gamma`         | `1.0`         | `> 0`                                  |
|               | `smooth`        | `0.5`         | `> 0`; `1.0` disables smoothing        |
|               | `brightness`    | `1.0`         | `0.0..=1.0`                            |
| `[behavior]`  | `on_no_session` | `"preset"`    | `keep-last\|black\|preset`             |
| `[capture]`   | `node_name`     | `"gamescope"` | non-empty `media.name` of the node     |

See [deploy/config.toml.example](./deploy/config.toml.example) for a fully commented example (the 65" strip: `top=86 right=48 bottom=86 left=48`, `start="bc"`, `direction="clockwise"`, `offset=0`).

### CLI overrides

One flag per config field, CLI-only flags aside: `--config PATH`, `--dry-run` (never send, log only), `--wled HOST`, `--port`, `--timeout`, `--top/--right/--bottom/--left`, `--start <8 codes>`, `--direction <clockwise|counterclockwise>`, `--offset N`, `--fps`, `--sample-height`, `--band`, `--saturation`, `--gamma`, `--smooth`, `--brightness`, plus debug-only `--once`, `--dump PNG`, `--preview`. Validation errors exit with a clap-style error (exit code 2); an explicit `--config` pointing at a missing or unparseable file is fatal at startup (exit code 2). Run `gamescope-led-sync --help` for the full list.

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

**Offset**: `offset > 0` sends the first `offset` physical LEDs as black (packet length `L = count + offset`); `offset < 0` rotates the chain (LED `i` gets zone `(i + |offset|) mod count`). Constraint: `L ≤ 490`.

**WLED segment-length rule**: the packet always starts at WLED LED index 0, so set your WLED segment length to `L = count + max(offset, 0)` (e.g. 268 for the 65" strip). A positive offset is sent as leading black LEDs.

## Decky plugin

`plugin/` is a [Decky Loader](https://github.com/SteamDeckHomebrew/decky-loader) plugin (no root flag; runs as the session user): service controls (enable at boot, start/stop/restart, logs, test one frame), WLED host/port, strip layout and offset slider, and the image sliders — every change saves (debounced 400 ms) and goes live at the daemon's next tick. `wled.timeout_s` and `behavior.on_no_session` stay file-only. See [plugin/README.md](./plugin/README.md).

## Protocol notes

WLED UDP realtime, protocol byte 2 (**DRGB**): a timeout byte (seconds WLED stays in realtime after the last packet), then three bytes per LED. One packet carries at most **490 LEDs**; beyond that, WLED's DNRGB multi-packet protocol would be needed — out of scope for v1.

While no gamescope session is connected, `on_no_session` decides what is sent: `preset` (default) sends nothing and WLED falls back to its own preset after `timeout_s`; `keep-last` resends the last colors every 1 s; `black` sends black every 1 s. During an active session a 1 s keepalive resends the last colors on static screens, so WLED never drops to its preset mid-game.

## Troubleshooting

- Logs: `journalctl --user -u gamescope-led-sync -f`.
- Capture one frame and save it: `gamescope-led-sync --once --dump frame.png` (the Decky plugin's _Test one frame_ does this and shows the PNG).
- See the edge colors live on a terminal: `--preview`.
- Never send anything, log only: `--dry-run`.
- Use a custom config file: `--config /path/to/config.toml`.

## Development

```sh
just ci        # fmt-check, clippy -D warnings, cargo test, markdown/toml/yaml/nix lints
cargo test
cargo run -- --help
```

The unit tests cover the geometry truth table (16 start × direction combos), offset cases, color grading, config parse/validate, CLI > file > default precedence, and live reload; `tests/cli.rs` covers the exit-code-2 contract and the `--help` surface at the process level.

Manual E2E checklist: `./deploy/install.sh --enable`; launch a game and verify the LEDs follow the screen edges; move the offset slider in the Decky plugin and see the shift within one frame; try `--once --dump` + `--preview`; try `--dry-run`.

## License

Copyright 2026 larry0x.

Licensed under the [Apache License, Version 2.0](./LICENSE).
