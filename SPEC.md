# SPEC — gamescope-led-sync

The implementation is complete and the code is self-documenting: doc comments
state the _why_ next to the _what_. This file records only what cannot be
inferred from the code — physical and domain choices, external contracts
(WLED, Decky Loader, systemd), cross-component agreements (journal markers,
version string), and repository conventions. Section numbers and decision IDs
(D1–D14) are **stable**: code comments and scripts reference them.

User-facing documentation lives in `README.md`; the configuration schema with
defaults and bounds lives in `src/config.rs` and the commented
`deploy/config.toml.example`.

## 1. Context

DIY Steam machine (CachyOS, SDDM with autologin, BC250): Steam runs in a
**gamescope** session and a `systemd --user` instance exists, so a user unit
is the autostart mechanism. An addressable strip (~268 LEDs around a 65" TV)
is driven by a **WLED** controller on the same LAN. The HDMI signal is never
touched: the daemon captures gamescope's own PipeWire stream, computes one
color per LED, and sends them to WLED as DRGB realtime UDP packets.
Deliverables: the Rust daemon, a TOML config file, a systemd user unit, and a
Decky Loader plugin (the install entry point). Details: `README.md`.

## 2. Non-obvious decisions

| # | Decision |
| - | -------- |
| D1 | The strip is a physical **loop** (the last LED is next to the first): `offset` is a circular shift, LED `i` gets zone `chain[(i − offset) mod count]`. The packet length is always `count`, whatever the sign. |
| D2 | ≤ **490 LEDs** (one DRGB packet, §12). DNRGB multi-packet is out of scope (§14). |
| D3 | **8 start points** (4 corners + 4 edge midpoints) × 2 directions **viewed from the front** = 16 combinations (§5.2). |
| D4 | Autostart via the user unit `gamescope-led-sync.service`, `WantedBy=default.target` (SDDM autologin ⇒ `systemd --user` exists). |
| D5 | `on_no_session = "preset"` (default): while disconnected and **unpaused**, send nothing and let WLED fall back to its preset after `timeout_s` (§6.1). |
| D6 | The Decky plugin is **non-root**: everything is user-space (`~/.local/bin`, `~/.config`), so `systemctl --user` works directly. |
| D7 | Plugin UI language: **English**. |
| D8 | Artifacts: daemon + user unit + Decky plugin zip. No Arch/AUR package (§14). |
| D9 | Plugin-exposed knobs: brightness, band_frac, smooth, fps, sample_height, plus the layout. `wled.timeout_s`, `wled.port`, `behavior.on_no_session`, `capture.node_name` stay **file-only**. |
| D10 | Test-one-frame exists as a backend method only (no UI button): `--once --dump`, PNG served from the plugin data dir (§10.1). |
| D11 | Offset slider range in the UI: **−20…+20, step 1**. |
| D12 | `[capture] start_paused`: the daemon starts paused when `true`; a reload applies it **only on an actual change of the key**, so a SIGUSR1 toggle is never clobbered (§6.2, §7). CLI override: `--start-paused`. |
| D13 | `gap` (0…=50) is a **hole at the loop seam** — between the strip's last and first LED (a cable duct) — in LED pitches. The hole has no LEDs: the seam edge is sampled as `n + gap` zones and the `gap` seam zones are dropped; the packet length stays `count` (§5.1, §5.3). |
| D14 | **One version X.Y.Z for the whole deliverable**, `Cargo.toml` as source of truth; `--version` prints exactly `gamescope-led-sync X.Y.Z` (§11). |

## 3. Where things live

- `src/` — the daemon (geometry, capture, output, reload, calibration mode).
- `deploy/` — the systemd user unit and the commented example config, embedded into the plugin payload.
- `scripts/` — packaging (`package-plugin.sh`) and dev/headless install (`install-all.sh`, `install-service.sh`).
- `plugin/` — the Decky plugin (Python backend + TS frontend).
- `tests/` — CLI contract tests.

## 4. Configuration

### 4.1 Location and precedence

1. `--config <PATH>`, else `$XDG_CONFIG_HOME/gamescope-led-sync/config.toml`, else `/etc/gamescope-led-sync/config.toml`, else built-in defaults (warning logged).
2. Value precedence: **CLI flag > config file > built-in default**. CLI-only flags (never in the file): `--config`, `--setup`, `--dry-run`, `--once`, `--dump`, `--preview`, `--preview-strip`, `--calibrate`.

### 4.2 Schema

Defined by the serde structs in `src/config.rs` (defaults and bounds inline)
and documented key by key in `deploy/config.toml.example`. No schema-version
key: unknown TOML keys are a hard error (§4.3), and a `version` key will be
introduced when the schema evolves.

### 4.3 Validation

One `Config::validate()` shared by the CLI and the file. Rules: every bound of
`src/config.rs`; `count = top+right+bottom+left` in 1…=490 (the gap has no LEDs
and does not count toward the packet); `start` / `direction` / `on_no_session`
must be known codes; a midpoint start on a 0-LED edge is rejected (§5.2);
**unknown keys are rejected** (`deny_unknown_fields`) so typos fail loudly.
Errors name the field, the value, and the bound. CLI validation exits with
code 2; an explicit `--config` pointing at a missing/unparseable file is fatal
(exit 2); a bad file at runtime keeps the previous config (§7).

### 4.4 CLI surface

One flag per config field, plus the CLI-only modes of §4.1. `--setup` is the
interactive first-run: asks for the WLED host (probed over HTTP
`GET /json/info`) and the strip layout, lights each edge in its own color to
check the counts, then writes the config and exits without starting the daemon.
`--preview-strip` sends the per-segment test pattern (bottom red, left blue,
top green, right yellow) until SIGTERM — the live preview of the plugin's
configuration screen. Full surface: `src/cli.rs` / `--help`.

## 5. Geometry conventions (`src/geometry.rs`)

### 5.1 Zones

`edge_means()` returns one zone color per LED, in canonical order (top and
bottom left→right, left and right top→bottom). With `gap > 0` (D13) the seam
edge is divided into `n + gap` equal zones and the `gap` zones at the seam
boundary are dropped, so no LED samples the screen region behind the hole.

### 5.2 Chain

The perimeter is a cyclic list of `count` zones. Base order: clockwise from
bottom-left (left edge bottom→top, top left→right, right top→bottom, bottom
right→left). `chain[0]` is the zone physically at the `start` point; `chain[i]`
is the i-th zone along `direction` (D3). The exact index math for all 16
start×direction combinations lives in the `geometry.rs` tests (truth table). A
midpoint start on a 0-LED edge is a validation error (§4.3); a corner start
with an empty adjacent edge degenerates to the next edge in rotation order.

### 5.3 Offset (D1)

`packet[i] = chain[(i − offset) rem_euclid count]`: `offset > 0` shifts toward
higher LED indices, `offset < 0` toward lower ones, wrapping around the loop.
The packet length is always `count`. `chain()` and `apply_offset()` are
separate; with `gap > 0` (D13) both are unchanged and wrapping across the seam
naturally jumps the hole.

### 5.4 Color grading

Per LED, on the chain **before** the offset: saturation → gamma → × brightness
→ clamp 0…255 → u8. Smoothing is an EMA per channel (`p += smooth·(n−p)`) on
the chain and resets when `count` changes. All grading is a no-op at defaults.

## 6. Daemon loop (one tick per `fps`)

### 6.0 Sender lifecycle

If `WledSender` creation fails (unresolvable host, bind error), the daemon
**never exits**: it runs senderless (like `--dry-run`) and retries every 5 s
and on every reload. Reason: with `Restart=on-failure`, an immediate exit
trips systemd's `StartLimitBurst` and locks the unit into `failed`. A reload
that switches to an unresolvable host warns and keeps the old sender.

### 6.1 Tick and no-session behavior (D5)

1. `stat()` the config file → maybe reload (§7).
2. Drain queued PipeWire buffers, keep the newest frame.
3. Fresh frame → `edge_means` → `chain` → smooth → grade → `apply_offset` → one DRGB packet; stored as `last_colors`.
4. Keepalive: if the stream is connected and no fresh frame for ≥ 1 s, resend `last_colors` (keeps WLED in realtime on static screens).
5. Stats log every 5 s.

While disconnected (no gamescope node / stream stopped / reconnect wait) and
unpaused: `keep-last` resends `last_colors` every 1 s, `black` sends black every
1 s, `preset` (default) sends nothing. If no frame was ever captured, all
three stay silent. On clean shutdown (SIGTERM) nothing is sent; WLED's
`timeout_s` applies.

### 6.2 Manual pause (SIGUSR1 / `start_paused`, D12) — journal-marker contract

SIGUSR1 toggles a manual pause; `[capture] start_paused = true` starts paused
the same way. While paused, the capture session ends cleanly and **nothing is
sent to WLED at all** — whatever `on_no_session` says — so WLED's realtime
timeout releases the strip and another owner can take over; §6.1 applies only
to an unpaused daemon. On resume the node is rediscovered and reconnected.

The daemon logs one line per transition, and **the exact marker strings are a
contract**: `plugin/main.py` and the calibration mode parse them from the
journal to derive the capture state. Markers: `found gamescope node`,
`session ended`, `retrying in 2 s`, `waiting for the gamescope node`,
`capture paused`, `capture resumed`.

Two capture clients on the same gamescope node at once are unsupported: the
plugin's test-frame method refuses to run while the service is active.

## 7. Live reload

Every tick, `stat()` the config file (mtime + len); if changed → read, parse,
validate:

- valid → atomically swap the shared config: recreate the sender if
  `wled.host`/`port` changed (`timeout_s` rides in every packet header, no
  recreation needed; an unresolvable new host keeps the old sender); reset
  smoothing if `count` changed; update the timer interval if `fps` changed;
  apply `start_paused` **only on an actual change of the key** (D12, so a
  SIGUSR1 toggle is not overridden by an unrelated reload). `sample_height` is
  read live each frame, but `requested_size` is negotiated at connect, so a new
  value is fully active after the next reconnect;
- invalid → warn with the field error, the **previous config stays active**;
- deleted/unreadable → keep the current config, one debug log on state change
  (no warn spam).

The plugin writes the file **atomically** (temp + `rename`), so the daemon
never reads a partial file; the effect is visible at the next tick.

## 8. systemd unit

`deploy/gamescope-led-sync.service`, installed to `~/.config/systemd/user/`:
`After=`/`Wants=pipewire.service`, `Restart=on-failure`, `RestartSec=2s`,
`WantedBy=default.target`. Enable/start: `systemctl --user enable --now
gamescope-led-sync`; logs: `journalctl --user -u gamescope-led-sync -f`. The
daemon waits for the gamescope node, so starting at boot before Steam is fine.

## 9. Install scripts (dev / headless path)

`scripts/install-service.sh` (`--binary`, `--enable`, `--uninstall`): builds
if needed, installs the binary to `~/.local/bin` (no sudo), copies the example
config **only if absent**, installs the unit, `daemon-reload`; `--uninstall`
keeps the config. `scripts/install-all.sh` = package + install the plugin via
the loader's local API + `install-service.sh --enable`. The main install path
is the plugin itself (§10.3).

## 10. Decky plugin (`plugin/`)

### 10.1 Backend contract (`main.py`)

- The loader runs as root and forks the backend without the user session
  environment, so every `systemctl --user` / `journalctl --user` subprocess
  must be spawned with `XDG_RUNTIME_DIR=/run/user/<uid>` and
  `DBUS_SESSION_BUS_ADDRESS=unix:path=...` filled in (`_session_env`).
- `capture_status` parses the §6.2 journal markers (last 200 lines) plus the
  live fps from the last stats line.
- `save_config` validates (Python mirror of §4.3) and writes the file
  **atomically**; the daemon applies it at the next tick (§7).
- `test_frame` (`--once --dump`) refuses while the service is active (§6.2) and
  serves the PNG at `/plugins/<plugin.json display name, URL-encoded>/data/last-frame.png`
  — the loader keys served URLs by **display name**, not by install folder.
- `preview_set` respawns the binary with `--preview-strip` and the draft layout
  as CLI flags, with `PR_SET_PDEATHSIG` so the preview dies with the plugin.

### 10.2 Frontend (`src/index.tsx`)

Two views: the main page (statuses, Capture toggles, Image sliders,
Configuration button — every control saves debounced via `save_config`) and the
configuration screen (pauses the capture on entry, live `--preview-strip`
preview on every draft change, nothing written until «Save and quit»). The
file-only keys (D9) are listed in `plugin/README.md`.

### 10.3 Install from the plugin (D14)

The distributed zip embeds `payload/` (binary + unit + example config +
LICENSE); the backend installs/updates/uninstalls from
`DECKY_PLUGIN_DIR/payload/`, all user-space (D6), idempotently, with a lock
against concurrent runs and a `.bak` rollback if an update fails mid-way. The
installed version comes from `~/.local/bin/gamescope-led-sync --version`; a
mismatch with `plugin.json` **in either direction** (downgrade included) means
«update available» and disables every other control of the page. Uninstall
keeps the config file.

## 11. Versioning and distribution (D14)

One version `X.Y.Z` shared by the whole deliverable; `Cargo.toml` is the source
of truth. `just bump-version X.Y.Z` syncs `plugin/plugin.json` +
`plugin/package.json`; `just check-versions` runs in `just ci` and fails on
drift. The binary's `--version` prints exactly `gamescope-led-sync X.Y.Z`
(contract test in `tests/cli.rs`) — the plugin parses this line to compare
versions (§10.3). `scripts/package-plugin.sh` (`just package-plugin`) builds
`dist/gamescope-led-sync-decky-<version>.zip` + `.sha256` with `payload/`
embedded (stripped release binary, unit, example config, LICENSE); CI verifies
the zip and uploads it as a workflow artifact. Target: x86_64 Linux glibc. Do
**not** edit the root `.gitignore`: `nix flake check` diffs it against the
tools repository at the pinned commit — `plugin/payload/` is ignored via
`plugin/.gitignore`, `dist/` via the existing root entry.

## 12. WLED protocol notes

- Realtime protocol **DRGB** (protocol byte 2), UDP port 21324 by default; one
  packet carries at most **490 LEDs** (`MAX_PACKET_LEDS`, D2).
- The WLED segment length must equal `count` and the packet always starts at
  WLED LED index 0 — offset and gap only shift inside the segment, so the
  segment length never changes.
- The timeout byte rides in every packet header, so changing `timeout_s` needs
  no reconnect.
- `release()` sends a header-only packet with timeout byte 0, three times (UDP
  loss): WLED leaves realtime mode immediately instead of waiting for
  `timeout_s`.
- WLED's white balance («Color white») is a per-channel gain that can only
  **attenuate** (255 = neutral) — the calibration fit of §13 relies on this.

## 13. Calibration mode (`--calibrate`)

Stand-alone, CLI-only mode (never persisted), independent from the daemon's
capture: fix strip/TV color drift by measuring with the eyes and deriving the
WLED white-balance values (§12). On start it pauses the daemon capture via
SIGUSR1 (§6.2) if the service is active, and a guard resumes it on exit — only
if this run paused it. Borderless full-screen window rendered in software,
driven by gamepad over evdev (plus a keyboard fallback); a fixed palette of 24
colors; each page shows the target full-screen and sends the slider-adjusted
color to the whole strip at **60% brightness** (DRGB keepalive every 250 ms).
Fit (`src/calibrate/fit.rs`): per channel, least squares
`gain = Σ(t·a)/Σ(t²)` over samples with `t > 0`, clamped to [0,1], normalized
so the max gain is 1.0 (white point at full brightness), emitted as WLED
R/G/B values `round(gain·255)`. The report is shown on a final page and
written to `~/.config/gamescope-led-sync/calibration-<UTC timestamp>.txt`. On
exit: `release()` (§12), then the resume guard. Launched from Steam Big Picture
via `deploy/gamescope-led-sync-calibrate.desktop`. Implementation:
`src/calibrate/`.

## 14. Out of scope

- More than 490 LEDs (WLED DNRGB multi-packet with start index).
- Multiple strips / multiple WLED controllers.
- A standalone web UI (the Decky plugin is the UI).
- An Arch package / AUR (D8).
- Autostart without `systemd --user` (not needed, D4; the fallback would be an XDG autostart `.desktop`).
- HDR-aware color science beyond saturation/gamma/brightness.
