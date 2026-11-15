# SPEC v1 — gamescope-led-sync as a SteamOS deliverable

Status: **locked after review with the project owner (2026-11-15)**. Every open
question has a recorded decision in §2. Anything not written here is out of
scope for v1 (§15).

## 1. Context and goals

- DIY Steam machine on CachyOS, Steam running in a **gamescope** session, BC250
  motherboard. Session started via **SDDM with autologin** → a `systemd --user`
  instance exists, so a **user unit** is the supported way to auto-start.
- An addressable LED strip runs around a **65" TV** (perimeter ≈ 4.5 m,
  ≈ 60 LEDs/m → **≈ 268 LEDs**), driven by a **WLED** controller on the same LAN.
- The HDMI signal is never touched (no splitter): the program captures
  gamescope's own PipeWire stream, computes one color per LED, and sends them
  to WLED as DRGB realtime UDP packets.
- v1 deliverables: the Rust daemon + config file + systemd user unit +
  `install.sh` + a **Decky Loader plugin** to configure everything live.

## 2. Locked decisions (from the review)

| # | Question | Decision |
| - | -------- | -------- |
| D1 | Offset semantics | `offset > 0`: the first `offset` physical LEDs are sent **black** (the «void at the start»), packet length = `offset + count`. `offset < 0`: rotation, LED `i` gets zone `(i + \|offset\|) mod count`. Constraint: `count + max(offset,0) ≤ 490`. |
| D2 | LED count / protocol cap | ≈ 270 LEDs (60/m). **490 max** (one DRGB packet) is a documented v1 constraint. DNRGB (>490, multi-packet) is out of scope v1. |
| D3 | Start points | **8 start points** `bl bc br rc tr tc tl lc` (4 corners + 4 edge midpoints) × `direction` ∈ {`clockwise`, `counterclockwise`} viewed from the front = 16 combinations, unit-tested. |
| D4 | Boot/session | SDDM + autologin → `systemd --user` available → user unit `gamescope-led-sync.service`, `WantedBy=default.target`. |
| D5 | Behavior with no gamescope session | `on_no_session = "preset"`: while disconnected, send **nothing** → WLED falls back to its own preset after `timeout_s`. (During an *active* session with a static screen, the 1 s keepalive still runs, so WLED never drops to preset mid-game.) |
| D6 | Decky plugin privileges | **Non-root** (no `"root"` flag): the backend runs as the session user → `systemctl --user …` and writing `~/.config/…` work directly. |
| D7 | Plugin UI language | **English**. |
| D8 | v1 artifacts | `install.sh` + systemd user unit + Decky plugin. No Arch package/AUR in v1. |
| D9 | Extra plugin parameters | Sliders: **brightness** (0–100 %), **band_frac** (2–20 %), **smooth** (0.05–1.0). Numeric: **fps** (10–60), **sample_height** (120–480, «Advanced»). |
| D10 | «Test one frame» button | Yes: runs the binary with `--once --dump`, the PNG is served by the plugin (`/plugins/<name>/data/last-frame.png`) and shown in a modal. |
| D11 | Offset slider range in the UI | **−30 … +30, step 1** (fixed range). |

Defaults chosen by default (flagged, correctable in the file): config example uses
the 65" strip `top=86 right=48 bottom=86 left=48` (268 LEDs), `start="bc"`
(owner's strip starts bottom-center), `direction="clockwise"`, `offset=0`.
`wled.timeout_s` (default 2) and `behavior.on_no_session` (default `"preset"`)
stay **file-only** — not exposed in the Decky UI (not selected in D9).

## 3. Repository layout (v1)

```
gamescope-led-sync/
  src/            # existing crate + new config.rs; geometry.rs extended
  deploy/
    gamescope-led-sync.service     # systemd user unit
    config.toml.example            # fully commented config
    install.sh                     # idempotent install/uninstall
  plugin/          # Decky plugin (official template: rollup + decky-frontend-lib + @decky/api)
    plugin.json  package.json  rollup.config.mjs  main.py  src/index.tsx  README.md
  SPEC.md          # this file
  README.md        # rewritten for the SteamOS deliverable
```

New Rust dependencies: `serde` + `toml` (config file). No file-watch crate: the
daemon `stat()`s the config file every tick. `clap` stays for the CLI.
Crate version: `0.1.0`.

## 4. Configuration file (TOML)

### 4.1 Location and precedence

1. `--config <PATH>` if given;
2. else `$XDG_CONFIG_HOME/gamescope-led-sync/config.toml` (`~/.config/…`);
3. else `/etc/gamescope-led-sync/config.toml`;
4. else built-in defaults (with a warning logged).

Value precedence: **CLI flag > config file > built-in default**.
CLI-only flags (never in the file): `--config`, `--dry-run`, `--once`, `--dump`,
`--preview`.

### 4.2 Schema, defaults, bounds

```toml
[wled]
host = "wled.local"     # hostname or IP, non-empty
port = 21324            # 1..65535
timeout_s = 2           # 1..255, sent in every DRGB packet header

[layout]
top = 86                # 0..490 each
right = 48
bottom = 86
left = 48
start = "bc"            # bl|bc|br|rc|tr|tc|tl|lc
direction = "clockwise" # clockwise|counterclockwise (viewed from the front)
offset = 0              # integer; packet length = count + max(offset,0) <= 490

[processing]
fps = 30                # 1..240 (UI: 10..60)
sample_height = 240     # 1..16384 (UI: 120..480)
band_frac = 0.08        # >0 and <1 (UI: 0.02..0.20)
saturation = 1.0        # >=0 (UI: 0.0..3.0, step 0.05)
gamma = 1.0             # >0 (UI: 0.5..2.5, step 0.05)
smooth = 0.5            # >0; 1.0 disables smoothing (UI: 0.05..1.0)
brightness = 1.0        # 0.0..1.0 (UI: 0..100 %)

[behavior]
on_no_session = "preset" # keep-last | black | preset  (D5)

[capture]
node_name = "gamescope"  # media.name of the gamescope PipeWire node; advanced, file-only
```

No schema-version key in v1: unknown keys are rejected (§4.3), and a `version`
key will be introduced when the schema evolves.

### 4.3 Validation (shared by CLI and file)

One `Config::validate()` used by both entry points. Rules: every bound above;
`count = top+right+bottom+left ≥ 1`; `count + max(offset, 0) ≤ 490`; `start` and
`direction` and `on_no_session` must be known codes; a midpoint `start` on an
edge with 0 LEDs is rejected (§5.2); **unknown TOML keys are a
hard error** (`deny_unknown_fields`) so typos cannot pass silently. Errors name
the field, the value, and the bound. CLI validation exits with a clap-style
error (exit code 2); an explicit `--config` pointing at a missing/unparseable
file is fatal at startup (exit code 2); a bad file at *runtime* is logged and
the previous config stays active (§7).

### 4.4 CLI surface (v1)

`--config PATH`, `--dry-run` (replaces «omit `--wled`»: never send, log only),
plus one flag per config field: `--wled HOST`, `--port`, `--timeout`,
`--top/--right/--bottom/--left`, `--start <8 codes>`, `--direction
<clockwise|counterclockwise>` (replaces `--counterclockwise`), `--offset N`,
`--fps`, `--sample-height`, `--band`, `--saturation`, `--gamma`, `--smooth`,
`--brightness`, and debug-only `--once`, `--dump PNG`, `--preview`.

## 5. LED strip geometry (§ geometry.rs)

### 5.1 Canonical zones (unchanged)

`edge_means()` returns per-edge zone colors in canonical order: top and bottom
left→right, left and right top→bottom. Zone count per edge = the configured
LED count of that edge.

### 5.2 Chain (§ `chain()`)

The perimeter is a cyclic list of `count` zones. Base order = clockwise from
bottom-left: left edge bottom→top, top left→right, right top→bottom, bottom
right→left (exactly the current `base` in `chain()`).

- `chain[0]` is the zone physically at the `start` point;
- `chain[i]` is the i-th zone along the strip in the chosen `direction`.

Formal definition (removes any index ambiguity). Let `nt, nr, nb, nl` be the
four edge counts and `count = nt+nr+nb+nl`. The base cyclic list (clockwise
from bottom-left) is:

- `base[0..nl)` = left edge walked bottom→top: `L[nl−1−k]` for `k` in `0..nl`
- `base[nl..nl+nt)` = top edge left→right: `T[k]`
- `base[nl+nt..nl+nt+nr)` = right edge top→bottom: `R[k]`
- `base[nl+nt+nr..count)` = bottom edge right→left: `B[nb−1−k]`

Start index `s` in the base list (clockwise: `chain[i] = base[(s+i) mod
count]`; counterclockwise: `chain[i] = base[(s−i) mod count]`):

| start | clockwise `s`      | counterclockwise `s`        |
| ----- | ------------------ | --------------------------- |
| bl    | 0                  | count−1                     |
| tl    | nl                 | nl−1                        |
| tr    | nl+nt              | nl+nt−1                     |
| br    | nl+nt+nr           | nl+nt+nr−1                  |
| lc    | floor(nl/2)        | nl−1−floor(nl/2)            |
| tc    | nl+floor(nt/2)     | nl+nt−1−floor(nt/2)         |
| rc    | nl+nt+floor(nr/2)  | nl+nt+nr−1−floor(nr/2)      |
| bc    | nl+nt+nr+floor(nb/2) | count−1−floor(nb/2)       |

Worked example (2 zones per edge, count = 8): base = `[L1,L0,T0,T1,R0,R1,B1,B0]`;
`bl` clockwise → s=0 → `L1,L0,T0,T1,R0,R1,B1,B0`; `bl` counterclockwise → s=7 →
`B0,B1,R1,R0,T1,T0,L0,L1`. Both reproduce the existing unit tests, which stay the
regression baseline, alongside the 16-combination truth-table tests.

Validation: a midpoint start on an edge with 0 LEDs is an error (e.g.
«start=tc requires top ≥ 1»); a corner start needs no extra constraint (with an
empty adjacent edge it degenerates to the next edge in rotation order).

### 5.3 Offset (D1)

`packet_length L = count + max(offset, 0)`, with `L ≤ 490` validated.
For `i in 0..L`:

- if `offset > 0` and `i < offset`: black `[0,0,0]`;
- else: `chain[(i - offset) rem_euclid count]`.

`chain()` returns the `count` zones; a separate `apply_offset()` builds the
final packet (black head + rotated chain). Both unit-tested (offset 0, +5, −5,
boundary `L = 490`, `L = 491` rejected).

### 5.4 Color grading order

Per LED, on the chain **before** offset padding: saturation gain → gamma →
`× brightness` → clamp 0..255 → u8. All no-ops at defaults (1.0 / 1.0 / 1.0).
Smoothing (EMA, `p += smooth·(n−p)` per channel) runs on the chain and resets
when `count` changes (existing length check).

## 6. Processing loop (per tick, rate = `fps`)

### 6.0 Sender lifecycle (locked)

- If `WledSender` creation fails (unresolvable host, bind error): **do not
  exit** — log a warning, run senderless (like `--dry-run`), and retry creation
  every 5 s and on every config reload. This avoids systemd's start-rate limit
  locking the unit into `failed` (`Restart=on-failure` + immediate exit would
  trip `StartLimitBurst`).
- If a reload changes `wled.host`/`port` to an unresolvable host: warn and keep
  the old sender (§7).
- Starting two capture clients on the same gamescope node at once is
  unsupported (see §10.1 `test_frame` guard and README troubleshooting).

### 6.1 Tick

1. `stat()` the config file (mtime + len) → maybe reload (§7).
2. Drain queued PipeWire buffers, keep the newest frame (unchanged).
3. If a fresh usable frame: `edge_means` → `chain` → smooth → grade →
   `apply_offset` → send one DRGB packet to WLED. Store as `last_colors`.
4. Mid-session keepalive: if stream connected and no fresh frame for ≥ 1 s,
   resend `last_colors` (keeps WLED in realtime on static screens) — unchanged.
5. Stats log every 5 s (unchanged).

Reconnect loop, signal handling, discovery (`media.name == "gamescope"`),
`requested_size` negotiation: **unchanged**.

### 6.1 No-session behavior (D5)

While disconnected (no gamescope node / stream stopped / reconnect wait):
`keep-last` → resend `last_colors` every 1 s (current `wait_and_keepalive`);
`black` → send black every 1 s; `preset` (default) → **send nothing**, WLED
falls back to its preset after `timeout_s`. If no frame was ever captured, all
three send nothing until the first frame. On clean shutdown (SIGTERM) the
daemon sends nothing; WLED's timeout applies.

## 7. Live reload (immediate changes, no restart)

Every tick, `fs::metadata(config_path)`; if mtime/len changed → read + parse +
validate:

- valid → atomically swap the shared `Rc<RefCell<Config>>`; recreate the
  `WledSender` if `wled.host`/`port` changed (`timeout_s` is per-packet, no
  recreation needed; if the new host does not resolve, warn and keep the old
  sender); reset smoothing if `count` changed; if `fps` changed, update the
  session timer interval; if the layout changed, log the layout line again;
  log each changed field at `info!`;
- invalid → `warn!` with the field error, **previous config stays active**;
- file deleted/unreadable → keep the current config, one debug log when the
  state changes (no warn spam).

`sample_height` note: the subsample step reads it live every frame;
`requested_size` is negotiated at connect, so a new `sample_height` is fully
active after the next reconnect.

The Decky plugin writes the file **atomically** (temp file + `rename`), so the
daemon never reads a partial file. Effect is visible at the next tick
(≤ 1/fps ≈ 33 ms at 30 fps). Implementation note: today `capture.rs` clones the
config into the timer closure and `main.rs` builds the sender once — both move
to shared `Rc<RefCell<…>>`.

## 8. systemd service (user unit)

`deploy/gamescope-led-sync.service`, installed to
`~/.config/systemd/user/gamescope-led-sync.service`:

```ini
[Unit]
Description=gamescope-led-sync — ambient LED backlight for gamescope (WLED)
After=pipewire.service
Wants=pipewire.service

[Service]
Type=simple
ExecStart=/usr/bin/gamescope-led-sync
Restart=on-failure
RestartSec=2s

[Install]
WantedBy=default.target
```

- Enable at boot / start now: `systemctl --user enable --now gamescope-led-sync`
- Stop: `systemctl --user stop gamescope-led-sync`; disable: `… disable …`
- Logs: `journalctl --user -u gamescope-led-sync -f`
- The daemon waits for the gamescope node (existing reconnect loop), so starting
  at boot before Steam is fine.

## 9. install.sh (idempotent)

```
install.sh [--binary PATH] [--enable]
install.sh --uninstall
```

- Build: `cargo build --release --locked` if cargo is available and no
  `--binary` given.
- Install binary → `/usr/bin/gamescope-led-sync` (`sudo install -m 0755`).
- Config: copy `deploy/config.toml.example` →
  `~/.config/gamescope-led-sync/config.toml` **only if absent** (never
  overwrite a user config).
- Unit: copy → `~/.config/systemd/user/gamescope-led-sync.service`, then
  `systemctl --user daemon-reload`.
- `--enable`: `systemctl --user enable --now gamescope-led-sync`.
- `--uninstall`: `systemctl --user disable --now`, remove unit + binary,
  `daemon-reload`. **The config file is kept** (documented).

## 10. Decky plugin (`plugin/`)

Based on the official decky plugin template (rollup, `decky-frontend-lib`,
`@decky/api` for `callBackend`). `plugin.json`: name «Gamescope LED Sync», no
`root` flag (D6), version 0.1.0. UI language: English (D7).

### 10.1 Backend (`main.py`, `class Plugin`)

Runs as the session user. Constants: `CONFIG =
~/.config/gamescope-led-sync/config.toml`, `BINARY =
/usr/bin/gamescope-led-sync`, unit name `gamescope-led-sync`. Methods:

| Method | Action |
| ------ | ------ |
| `get_config()` | Parse the TOML (`tomllib`), return the dict (defaults merged). |
| `save_config(cfg)` | Validate (Python mirror of §4.3), write **atomically** (tmp + `rename`), return `{ok, errors}`. The daemon applies it at the next tick (§7). |
| `service_status()` | `systemctl --user show gamescope-led-sync -p ActiveState,SubState,UnitFileState` (+ `is-enabled`). |
| `service_set_enabled(bool)` | `systemctl --user enable` / `disable`. |
| `service_start/stop/restart()` | `systemctl --user …`. |
| `get_logs(n=200)` | `journalctl --user -u gamescope-led-sync -n N --no-pager`. |
| `test_frame()` | Refuses when the service is active (two capture clients on one gamescope node are unsupported, §6.0): `{ok:false, error:"stop the service first"}`. Otherwise runs `[BINARY, "--config", CONFIG, "--once", "--dump", <plugin data dir>/last-frame.png]` (timeout 30 s; clear error if the binary is missing) and returns `{ok, log}`. PNG served at `/plugins/<name>/data/last-frame.png` (plugin data dir from the loader's `DECKY_*` env vars). |

### 10.2 Frontend (`src/index.tsx`) — control mapping

- **Service** section: `ToggleField` «Enabled at boot» ↔ `service_set_enabled`;
  status line (`ActiveState`/`SubState`); `ButtonItem`s Start / Stop / Restart /
  View logs (modal with `get_logs`) / Test frame (D10: modal with
  `<img src="/plugins/gamescope-led-sync/data/last-frame.png?ts=…">`).
- **WLED**: `TextField` host; numeric `TextField` port (1–65535).
- **LED strip**: four numeric `TextField`s top/right/bottom/left (0–490, sum
  constraint checked on save); `DropdownField` start (8 labelled options);
  `DropdownField` direction (Clockwise / Counterclockwise, viewed from the
  front); `SliderField` offset, **min −30, max +30, step 1** (D11), value shown.
- **Image**: `SliderField` saturation (0–3, step 0.05), gamma (0.5–2.5, step
  0.05), brightness (0–100 %), band (2–20 %), smooth (0.05–1.0, step 0.05);
  numeric fps (10–60); «Advanced»: numeric sample_height (120–480).
- Saving: every control saves on change, **debounced 400 ms**, via
  `save_config`; a status line shows «Saved ✓» or the validation errors (the
  file is not written when invalid). No separate Apply button — changes are
  live (daemon reloads at the next tick).
- A hint line shows the computed packet length `L = count + max(offset, 0)`
  («set your WLED segment length to L») and is highlighted whenever L changes
  (§11).
- `wled.timeout_s` and `behavior.on_no_session`: file-only (D9/D5), documented
  in the plugin README.

## 11. WLED prerequisites (documented in README)

- UDP realtime enabled, protocol **DRGB**, port 21324 (or as configured).
- Segment length = `L = count + max(offset, 0)` (e.g. 268) — the packet always
  starts at WLED LED index 0; a positive offset is sent as leading black LEDs.
- PC and WLED on the same LAN; `wled.local` resolvable (mDNS) or use an IP.

## 12. Tests

- Existing `geometry.rs` / `output.rs` unit tests kept; corner tests ported to
  `StartPoint` and used as the regression baseline for the 16 combos.
- New: 16-combination start×direction truth table (against the §5.2 index
  table); midpoint split semantics; offset cases (0, +n, −n, `L=490` ok,
  `L=491` rejected); grading order incl. brightness; DRGB packet with black
  head; config TOML parse/validate (one test per error class); CLI>file>default
  merge precedence; reload apply (valid swap / invalid keeps old).
- `--dump` / `--preview` grade with saturation + gamma + brightness, so the
  diagnostics match the LED output exactly.
- New files must pass the existing `just` recipes (markdownlint on SPEC.md and
  READMEs, yamllint on the unit file, tombi on TOML, biome via the plugin
  template).
- Manual E2E checklist in README: `install.sh --enable`, launch a game, verify
  LEDs; move the offset slider in Decky and see the shift within one frame;
  `--once --dump` + `--preview`; `--dry-run`.

## 13. README rewrite (v1)

Objective (65" TV, CachyOS/gamescope/BC250, WLED), the 4-step pipeline (kept),
requirements (Linux, PipeWire user session, systemd --user, WLED with DRGB,
≤ 490 LEDs), install, configuration reference (TOML table + CLI overrides),
service management (systemctl --user + Decky), strip geometry (8 start points,
direction, offset semantics, WLED segment-length rule), Decky usage, protocol
notes (DRGB, 490 cap, DNRGB = future), troubleshooting (journalctl, --once
--dump, --dry-run), development (`just ci`, tests). Remove the dead `../python`
reference from `main.rs` doc comments and the «100-inch TV» wording.

## 14. Dead code / cleanups folded into v1

- `src/main.rs:8` doc reference to `../python` (absent) → delete.
- `README.md` «early WIP» / 100-inch → rewrite (§13).
- `Cargo.toml` version `0.0.0` → `0.1.0`.
- Validation factored out of clap attributes into `Config::validate()` (§4.3).
- `--counterclockwise` → `--direction`; `--start` gains the 4 midpoint codes;
  add `--offset`, `--brightness`, `--dry-run` (§4.4).
- `main.rs` no longer exits when `WledSender::new` fails — senderless retry
  instead (§6.0); same for an unresolvable host after a reload (§7).

## 15. Out of scope for v1

- > 490 LEDs (WLED DNRGB multi-packet with start index) — revisit if the strip
  grows.
- Multiple strips / multiple WLED controllers.
- A standalone web UI (Decky plugin is the UI).
- Arch package / AUR (D8).
- Autostart without `systemd --user` (not needed: SDDM autologin, D4). If that
  ever changes, fallback = XDG autostart `.desktop`.
- HDR-aware color science beyond saturation/gamma/brightness.

## 16. Audit trail — second pass (2026-11-15)

The spec was re-read as an implementer; the following details were ambiguous and
are now locked where indicated:

1. Exact chain index convention for all 8 start points × 2 directions → §5.2
   formal index table (verified against the existing corner tests).
2. Midpoint start on an edge with 0 LEDs → validation error (§5.2/§4.3); corner
   start degenerates gracefully.
3. WLED sender unresolvable at startup → senderless retry every 5 s, never exit
   (§6.0); unresolvable after reload → keep old sender (§7).
4. Explicit `--config` to a missing/unparseable file → fatal, exit code 2 (§4.3).
5. Config file deleted at runtime → keep current config, debug log only (§7).
6. `fps` change at runtime → session timer interval updated (§7);
   `sample_height` live for the step, at next reconnect for `requested_size` (§7).
7. `test_frame` while the service is active → refused (two capture clients on
   one gamescope node are unsupported) (§6.0, §10.1).
8. WLED segment-length hint: the plugin shows `L = count + max(offset,0)` and
   highlights it when it changes (§10.2, §11).
9. `--dump`/`--preview` grade with saturation + gamma + brightness (§12).
10. No config schema-version key in v1; unknown keys are hard errors (§4.2).
11. `[capture] node_name = "gamescope"` added as an advanced, file-only escape
    hatch in case a setup names its PipeWire node differently (§4.2).
12. Numeric fields in Decky = `TextField` with numeric validation (decky-frontend-lib
    has no dedicated number field) — already in §10.2, restated here.
13. New files must pass the existing lint/format recipes (§12).
