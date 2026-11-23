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
| D1 | Offset semantics | The strip is a **loop** (the last LED is physically next to the first): `offset` is a **circular shift** of the whole strip, LED `i` gets zone `chain[(i − offset) mod count]`. `offset > 0` shifts toward higher LED indices (right along the strip), `offset < 0` toward lower ones (left), wrapping around. Packet length = `count` always (≤ 490), so the offset — sign change included — never alters the packet length and cannot break the WLED realtime stream. |
| D2 | LED count / protocol cap | ≈ 270 LEDs (60/m). **490 max** (one DRGB packet) is a documented v1 constraint. DNRGB (>490, multi-packet) is out of scope v1. |
| D3 | Start points | **8 start points** `bl bc br rc tr tc tl lc` (4 corners + 4 edge midpoints) × `direction` ∈ {`clockwise`, `counterclockwise`} viewed from the front = 16 combinations, unit-tested. |
| D4 | Boot/session | SDDM + autologin → `systemd --user` available → user unit `gamescope-led-sync.service`, `WantedBy=default.target`. |
| D5 | Behavior with no gamescope session | `on_no_session = "preset"`: while disconnected, send **nothing** → WLED falls back to its own preset after `timeout_s`. (During an _active_ session with a static screen, the 1 s keepalive still runs, so WLED never drops to preset mid-game. A manual pause, §6.2, always releases the WLED — nothing is sent while paused.) |
| D6 | Decky plugin privileges | **Non-root** (no `"root"` flag): the backend runs as the session user → `systemctl --user …` and writing `~/.config/…` work directly. |
| D7 | Plugin UI language | **English**. |
| D8 | v1 artifacts | `install.sh` + systemd user unit + Decky plugin. No Arch package/AUR in v1. |
| D9 | Extra plugin parameters | Sliders: **brightness** (0–100 %), **band_frac** (1–20 %), **smooth** (0.05–1.0). Numeric: **fps** (10–60), **sample_height** (120–480, «Advanced»). |
| D10 | «Test one frame» button | Backend method only (no UI button since the ticket-10 refonte): runs the binary with `--once --dump`, the PNG is served by the plugin (`/plugins/<name>/data/last-frame.png`). |
| D11 | Offset slider range in the UI | **−20 … +20, step 1** (fixed range; ticket 10 narrowed it from −30…+30). |
| D12 | Capture at startup | `[capture] start_paused = false` (bool): the daemon starts paused when `true` (the Decky «Capture at startup» switch); a change of the key also applies **immediately** on live reload (§7). CLI override: `--start-paused`. |
| D13 | Seam hole (`gap`, ticket 14) | `gap` (0..=50, default 0) is a **hole at the loop seam**, in LED pitches, between the strip's **last and first LED** (owner's creux: a cable duct ≈ 7 LEDs wide at the bottom center, with `start="bc"`). The hole lies on the **seam edge**: that edge is divided into `n+gap` zones and the `gap` zones at the seam boundary are dropped, so no LED samples the screen region behind the creux. The packet length **stays `count`** (the WLED segment holds only real LEDs); `chain()`/`apply_offset()` are unchanged — the offset still rotates the `count`-LED chain, and wrapping across the seam naturally jumps the hole. UI: slider **0..50, step 1** in the configuration screen. |

Defaults chosen by default (flagged, correctable in the file): config example uses
the 65" strip `top=86 right=48 bottom=86 left=48` (268 LEDs), `start="bc"`
(owner's strip starts bottom-center), `direction="clockwise"`, `offset=0`, `gap=0`.
`wled.timeout_s` (default 2) stays **file-only** — not exposed in the Decky
UI (not selected in D9). `behavior.on_no_session` (default `"preset"`) and
`capture.start_paused` (default `false`, D12) are file-only since the
ticket-10 refonte; the Decky plugin exposes the startup switch but not the
no-session dropdown.

## 3. Repository layout (v1)

```text
gamescope-led-sync/
  src/            # existing crate + new config.rs; geometry.rs extended
  deploy/
    gamescope-led-sync.service     # systemd user unit
    config.toml.example            # fully commented config
    install.sh                     # idempotent install/uninstall
  plugin/          # Decky plugin (official template: @decky/rollup + @decky/ui + @decky/api, install.sh)
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
CLI-only flags (never in the file): `--config`, `--setup`, `--dry-run`, `--once`,
`--dump`, `--preview`, `--preview-strip`.

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
offset = 0              # integer; circular shift of the strip loop, packet length stays count <= 490
gap = 0                 # 0..=50; hole at the loop seam (between the last and first LED) in LED pitches, skipped when sampling the screen; packet length stays count

[processing]
fps = 30                # 1..240 (UI: 10..60)
sample_height = 240     # 1..16384 (UI: 120..480)
band_frac = 0.08        # >0 and <1 (UI: 0.01..0.20)
saturation = 1.0        # >=0 (UI: 0.0..3.0, step 0.05)
gamma = 1.0             # >0 (UI: 0.5..2.5, step 0.05)
smooth = 0.5            # >0; 1.0 disables smoothing (UI: 0.05..1.0)
brightness = 1.0        # 0.0..1.0 (UI: 0..100 %)

[behavior]
on_no_session = "preset" # keep-last | black | preset  (D5)

[capture]
node_name = "gamescope"  # media.name of the gamescope PipeWire node; advanced, file-only
start_paused = false     # bool (D12): start with the capture paused; Decky «Capture at startup»
```

No schema-version key in v1: unknown keys are rejected (§4.3), and a `version`
key will be introduced when the schema evolves.

### 4.3 Validation (shared by CLI and file)

One `Config::validate()` used by both entry points. Rules: every bound above
(incl. `layout.gap` **0..=50**, D13); `count = top+right+bottom+left ≥ 1` and
`≤ 490` (the gap is a hole without LEDs, so it does not count toward the
packet); `start` and
`direction` and `on_no_session` must be known codes; a midpoint `start` on an
edge with 0 LEDs is rejected (§5.2); **unknown TOML keys are a
hard error** (`deny_unknown_fields`) so typos cannot pass silently. Errors name
the field, the value, and the bound. CLI validation exits with a clap-style
error (exit code 2); an explicit `--config` pointing at a missing/unparseable
file is fatal at startup (exit code 2); a bad file at _runtime_ is logged and
the previous config stays active (§7).

### 4.4 CLI surface (v1)

`--config PATH`, `--dry-run` (replaces «omit `--wled`»: never send, log only),
plus one flag per config field: `--wled HOST`, `--port`, `--timeout`,
`--top/--right/--bottom/--left`, `--start <8 codes>`, `--direction
<clockwise|counterclockwise>` (replaces `--counterclockwise`), `--offset N`, `--gap N`,
`--fps`, `--sample-height`, `--band`, `--saturation`, `--gamma`, `--smooth`,
`--brightness`, `--start-paused` (forces `[capture] start_paused = true`),
and debug-only `--once`, `--dump PNG`, `--preview`, `--preview-strip`
(sends the per-segment test pattern — bottom red, left blue, top green,
right yellow — to WLED until SIGTERM; used by the Decky plugin's
configuration screen for the live preview; on a midpoint start edge the end
of the strip is violet, so the loop's seam is visible), plus
`--setup`: interactive first-run setup that asks for the WLED host and the
strip layout (LED counts per edge, start point, direction), probes the
controller over HTTP (`GET /json/info`) when the host is entered, lights
each edge in its own color to check the counts, lets the user go back and
modify any setting, and finally writes the config file (default lookup
path, or `--config PATH`) with the other parameters left at their defaults;
it exits without starting the daemon.

## 5. LED strip geometry (§ geometry.rs)

### 5.1 Canonical zones (with the seam hole, D13)

`edge_means()` returns per-edge zone colors in canonical order: top and bottom
left→right, left and right top→bottom. Zone count per edge = the configured
LED count of that edge.

With `gap > 0` (D13), the **seam edge** (the start edge for a midpoint start;
for a corner start the edge whose base-list run contains the seam boundary)
is divided into `n + gap` equal zones and the `gap` zones at the seam boundary
— the creux between the strip's last and first LED — are **dropped**
(`seam_hole()`), so the edge still yields exactly `n` zones and no LED samples
the screen region behind the creux. With `gap = 0` every zone of every edge is
sampled, exactly as before.

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

The strip is a **loop**: the last LED is physically next to the first, so the
offset is a circular shift of the whole strip, whatever its sign. For
`i in 0..count`: `packet[i] = chain[(i - offset) rem_euclid count]`.
`offset > 0` shifts the image toward higher LED indices (right along the
strip), `offset < 0` toward lower indices (left), wrapping around the loop.
The packet length is always `count` (validated `≤ 490`), so the offset never
changes it.

`chain()` returns the `count` zones; a separate `apply_offset()` builds the
final packet (the circularly shifted chain). Both unit-tested (offset 0, +n,
−n, `|offset| > count` wraps, `count = 490` ok, `491` rejected).

With `gap > 0` (D13) the chain and the offset are unchanged: the offset still
rotates the `count`-LED chain, and because the seam edge's kept zones sit at
the creux edges, wrapping across the loop seam naturally jumps the hole. The
packet length stays `count` — the hole has no LEDs.

### 5.4 Color grading order

Per LED, on the chain **before** the circular offset: saturation gain → gamma →
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

Reconnect loop, discovery (`media.name == "gamescope"`),
`requested_size` negotiation: **unchanged**.

### 6.1 No-session behavior (D5)

While disconnected (no gamescope node / stream stopped / reconnect wait — but
not while manually paused, §6.2):
`keep-last` → resend `last_colors` every 1 s (current `wait_and_keepalive`);
`black` → send black every 1 s; `preset` (default) → **send nothing**, WLED
falls back to its preset after `timeout_s`. If no frame was ever captured, all
three send nothing until the first frame. On clean shutdown (SIGTERM) the
daemon sends nothing; WLED's timeout applies.

### 6.2 Manual capture pause (SIGUSR1, ticket 09; WLED release: ticket 11) and startup pause (D12, ticket 10)

SIGUSR1 toggles a manual pause (the Decky plugin's Capture switch).
`[capture] start_paused = true` starts the daemon paused the same way, and
a reload that changes the key applies it immediately (pause or resume).
While paused, the daemon ends any running capture session
cleanly (the stream disconnects through its destructor, exactly like a
normal session end) and stays in the reconnect wait, with live reload
still working; no capture runs. While paused, «nothing is sent to the
WLED at all» — whatever `on_no_session` says (ticket 11) — so WLED's
realtime timeout releases the strip after the last packet's `timeout_s`
and another owner (its own preset, another app) can take over. §6.1
applies only to the no-session wait of an «unpaused» daemon. On resume
it rediscovers the node and reconnects. Each transition is logged once («capture paused
(SIGUSR1)… » / «capture resumed (SIGUSR1)», plus «capture paused at
startup ([capture] start_paused = true)») so the plugin can parse the
state from the journal (§10.1 `capture_status`).

## 7. Live reload (immediate changes, no restart)

Every tick, `fs::metadata(config_path)`; if mtime/len changed → read + parse +
validate:

- valid → atomically swap the shared `Rc<RefCell<Config>>`; recreate the
  `WledSender` if `wled.host`/`port` changed (`timeout_s` is per-packet, no
  recreation needed; if the new host does not resolve, warn and keep the old
  sender); reset smoothing if `count` changed; if `fps` changed, update the
  session timer interval; if `capture.start_paused` changed, apply the pause
  immediately (`capture::set_paused`, D12 — only on an actual change of the
  key, so a SIGUSR1 toggle is not overridden by an unrelated reload);
  if the layout changed, log the layout line again;
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

```text
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

Based on the current official decky plugin template (`@decky/rollup` +
`@decky/ui`, `@decky/api` for `callable`/`definePlugin`; Decky Loader 3.x loads
`dist/index.js` as an ES module because `package.json` has `"type": "module"`,
and `callBackend` no longer exists at runtime). `plugin.json`: name
«Gamescope LED Sync», no `root` flag (D6), `api_version: 1` (instance methods,
positional backend args), version 0.1.0. Install with `plugin/install.sh`
(builds the frontend, then installs through the loader's local API — no sudo;
`--direct` copies to `~/homebrew/plugins/gamescope-led-sync` with sudo and
restarts `plugin_loader`). UI language: English (D7).

### 10.1 Backend (`main.py`, `class Plugin`)

Runs as the session user. The loader runs as root and forks the backend
with the root environment, so `XDG_RUNTIME_DIR` and
`DBUS_SESSION_BUS_ADDRESS` are missing and every `systemctl --user` /
`journalctl --user` call would fail ("Failed to connect to user scope
bus") — the backend fills them in (`_session_env()`, pointing at
`/run/user/<uid>`) for each `--user` subprocess. Constants: `CONFIG =
~/.config/gamescope-led-sync/config.toml`, `BINARY =
/usr/bin/gamescope-led-sync`, unit name `gamescope-led-sync`. Methods:

| Method | Action |
| ------ | ------ |
| `get_config()` | Parse the TOML (`tomllib`), return the dict (defaults merged). |
| `save_config(cfg)` | Validate (Python mirror of §4.3), write **atomically** (tmp + `rename`), return `{ok, errors}`. The daemon applies it at the next tick (§7). Used by the main page (debounced) and by the configuration screen's «Save and quit». |
| `service_status()` | `systemctl --user show gamescope-led-sync -p ActiveState,SubState,UnitFileState` (+ `is-enabled`); returns `{ok, ActiveState, SubState, UnitFileState, enabled, error?}` — `error` carries the `systemctl` stderr when the `show` call fails (empty stdout would otherwise read as `unknown`). |
| `service_set_enabled(bool)` | `systemctl --user enable` / `disable`. |
| `service_start/stop/restart()` | `systemctl --user …`. |
| `capture_status()` | Service ActiveState plus the capture phase parsed from the last 200 journal lines: `capturing` (last «found gamescope node»), `waiting` («session ended»/«retrying»/«waiting for the gamescope node»/«capture resumed»), `paused` («capture paused»), `inactive` (service not active); also the live fps from the last stats line. Returns `{ok, active, state, fps, detail}`. |
| `capture_set_paused(bool)` | Manual capture pause via `systemctl --user kill --signal=SIGUSR1 gamescope-led-sync` (§6.2). Refuses when the service is not active; only signals when the parsed state differs from the request. Returns `{ok, error?}`. |
| `probe_wled(host)` | Reachability check: HTTP `GET /json/info` on port 80 (same probe as `--setup`), 3 s timeout. Returns `{ok, info?}` / `{ok:false, error}`. Drives the green OK / red NOK next to the host field. |
| `wled_status()` | `probe_wled` on the configured `wled.host`; returns `{ok, configured, host?, reachable, info?, error?}`. Strip status line of the main page. |
| `preview_set(host, port, timeout_s, layout)` | Validate the draft layout (Python mirror of §4.3), then (re)spawn `BINARY --preview-strip --wled host --port port --timeout S --top … --right … --bottom … --left … --start … --direction … --offset … --gap …` (the draft layout as CLI flags overrides the file, §4.1) with `PR_SET_PDEATHSIG` so it dies with the plugin. Returns `{ok, packet_len}` or `{ok:false, errors}`; an invalid draft is not spawned. |
| `preview_stop()` | Terminate the preview process. Called when leaving the configuration screen. |
| `get_logs(n=200)` | `journalctl --user -u gamescope-led-sync -n N --no-pager`. (No UI button since the ticket-10 refonte.) |
| `test_frame()` | Refuses when the service is active (two capture clients on one gamescope node are unsupported, §6.0): `{ok:false, error:"stop the service first"}`. Otherwise runs `[BINARY, "--config", CONFIG, "--once", "--dump", <plugin data dir>/last-frame.png]` (timeout 30 s; clear error if the binary is missing) and returns `{ok, log}`. PNG written to `DECKY_PLUGIN_RUNTIME_DIR` and served at `/plugins/<plugin.json name, URL-encoded>/data/last-frame.png` — the loader keys served URLs by display name, not by install folder. (No UI button since the ticket-10 refonte.) |

### 10.2 Frontend (`src/index.tsx`) — two views (ticket 10)

One `Content` component with `mode: "main" | "config"`.

**Main page** — only these controls:

- **Status** section (refreshed on mount and every 5 s): `Field` service
  (`ActiveState`/`SubState` + enabled), `Field` capture (`capture_status`
  state + detail), `Field` FPS (live `capture_status.fps`), `Field` WLED
  strip (`wled_status`: reachable / not reachable / not configured, with the
  probe detail).
- **Capture** section: `ToggleField` «Capture» ↔ `capture_set_paused`
  (SIGUSR1, immediate, runtime-only; disabled when the service is not
  active); `ToggleField` «Capture at startup» ↔ `capture.start_paused`
  (D12), saved via `save_config` — immediate effect on reload and persisted
  for boot.
- **Image** section: `SliderField` brightness (0–100 %), gamma (0.5–2.5,
  step 0.05), saturation (0–3, step 0.05), smooth (0.05–1.0, step 0.05).
- **Configuration** section: `ButtonItem` «Configuration» → configuration
  screen.
- Saving on the main page: every control saves on change, **debounced
  400 ms**, via `save_config` — the effect is live at the daemon's next tick
  and the value is reloaded at boot. No status line (all main-page controls
  are within the §4.3 bounds, so a save cannot fail).

**Configuration screen** — entered via the Configuration button:

- On entry the capture is **paused** (`capture_set_paused(true)`, only if
  not already paused; remembered to resume on exit), and `preview_set` is
  called with the current config: the strip lights each segment in its own
  color — **bottom red, left blue, top green, right yellow** — via the
  daemon's `--preview-strip` mode, so the user sees the segments while
  editing their lengths. When the start point is an **edge midpoint** (ticket
  13), that edge is split at the loop's seam: the beginning of the strip
  keeps the edge color, the end is **violet**, so the two halves of the seam
  edge read differently and the circular offset is visible on the strip.
- Every draft change calls `preview_set` again (debounced 300 ms; the
  backend respawns the preview process with the new layout as CLI flags).
  **Nothing is written to the config file until «Save and quit»** — the
  immediate effect on the strip goes exclusively through the preview.
- Fields: `TextField` WLED host/IP + a «Host check» `Field` showing a green
  **OK** / red **NOK** from `probe_wled` (probed on change, debounced 600 ms);
  `DropdownItem` start point (8 **full-word** labels, no codes);
  `ToggleField` clockwise rotation (on = clockwise, off = counterclockwise);
  four numeric `TextField`s for the segment lengths (bottom, left, top,
  right), each flanked by **−1 / +1** `ButtonItem`s that step the count
  within the §4.3 bounds (no keyboard needed); a read-only `Field` with the
  total active LED count
  (`top + right + bottom + left = N`, recomputed live); `SliderField`
  offset **−20 … +20, step 1** (D11); `SliderField` **gap 0 … 50, step 1**
  (D13: hole at the loop seam between the last and first LED, in LED pitches;
  the creux is skipped when sampling the screen, the packet length stays the
  active count); `SliderField` screen band **1–20 %**;
  numeric `TextField` FPS.
- «Save and quit»: `save_config(draft)` (validation errors are shown and the
  screen stays), `preview_stop`, resume the capture if we paused it, back to
  the main page. «Cancel»: `preview_stop`, resume if needed, back to the main
  page, config file untouched.
- `wled.timeout_s`, `wled.port`, `sample_height`, `behavior.on_no_session`,
  `capture.node_name`: file-only (D9/D12), documented in the plugin README.

## 11. WLED prerequisites (documented in README)

- UDP realtime enabled, protocol **DRGB**, port 21324 (or as configured).
- Segment length = `count` (e.g. 268) — the packet always starts at WLED LED
  index 0 and always carries the whole chain; the offset only shifts it
  circularly within the segment, and the `gap` hole (D13) has no LEDs so it
  does not change the segment length either.
- PC and WLED on the same LAN; `wled.local` resolvable (mDNS) or use an IP.

## 12. Tests

- Existing `geometry.rs` / `output.rs` unit tests kept; corner tests ported to
  `StartPoint` and used as the regression baseline for the 16 combos.
- New: 16-combination start×direction truth table (against the §5.2 index
  table); midpoint split semantics; offset cases (0, +n, −n, wrap-around
  `|offset| > count`, `count=490` ok, `491` rejected); gap cases (D13:
  `gap` 0..=50, `51` rejected, packet length stays `count`, seam-hole position
  per start/direction, seam-edge sampling skips the creux, chain seam at the
  creux edges); grading order incl.
  brightness; config TOML parse/validate (one test per error class);
  CLI>file>default merge precedence; reload apply (valid swap / invalid keeps
  old);
  ticket 10: `[capture] start_paused` (file parse, `--start-paused` override,
  reload applies the pause flag on change) and the `--preview-strip`
  segment pattern (bottom red / left blue / top green / right yellow through
  `chain_edges` + `apply_offset`, incl. circular offset and the midpoint-seam
  split: head run keeps the edge color, tail run is violet).
- `--dump` / `--preview` grade with saturation + gamma + brightness, so the
  diagnostics match the LED output exactly.
- New files must pass the existing `just` recipes (markdownlint on SPEC.md and
  READMEs, yamllint on the unit file and the workflows, tombi on TOML, and —
  for the Decky plugin — biome, `tsc`, the rollup build, and a `py_compile`
  of `main.py`, via the js/python recipe overrides in the justfile). CI runs
  them through the GitHub Actions workflows in `.github/workflows/` (rust,
  markdown, nix, toml, yaml, js, python), which call the reusable workflows
  of the tools repository pinned in flake.lock.
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
8. WLED segment-length hint: the plugin shows `L = count` (the offset is a
   circular shift, so the packet length never changes) and highlights it when
   it changes (§10.2, §11).
9. `--dump`/`--preview` grade with saturation + gamma + brightness (§12).
10. No config schema-version key in v1; unknown keys are hard errors (§4.2).
11. `[capture] node_name = "gamescope"` added as an advanced, file-only escape
    hatch in case a setup names its PipeWire node differently (§4.2).
12. Numeric fields in Decky = `TextField` with numeric validation (@decky/ui
    has no dedicated number field) — already in §10.2, restated here.
13. New files must pass the existing lint/format recipes (§12).
14. Below the four per-edge LED count fields, the plugin shows a read-only,
    non-editable total LED count (`top + right + bottom + left`), recomputed
    live (§10.2).
