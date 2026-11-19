# 10: Decky plugin refonte — two-page UI (main + configuration), start_paused, live strip preview

**What to build:** Refactor the Decky plugin into two views, per the owner's spec
(2026-11-20). Decisions locked with the owner:

- «backup» in the original request was a typo for **backend** — no backup mechanism
  exists or is needed; the changes affect `plugin/main.py`.
- «Capture at startup» is persisted as a new config key **`[capture] start_paused`
  (bool, default false)** in the TOML; the daemon starts paused when `true`, and the
  key also takes effect immediately on live reload. CLI flag `--start-paused`.
- Entering the configuration screen **auto-pauses capture** (SIGUSR1, only if not
  already paused; resumed on exit only if we paused it), so the preview owns the strip.
- Preview segment colors: **bottom = red, left = blue, top = green, right = yellow**.
- Configuration screen has **two exit buttons**: «Save and quit» (writes the config
  file) and «Cancel» (leaves without writing).
- UI language stays **English** (D7 unchanged).
- The main page loses: service start/stop/restart, view logs, test one frame, WLED
  host/port fields, layout fields, offset, band, fps, sample_height, on_no_session
  dropdown (still available via CLI/config file).
- Band slider range becomes **1–20 %** and the offset slider **−20…+20** (SPEC D11 and
  §10.2 updated accordingly).

## Main page (only these controls)

- Status: service (`ActiveState`/`SubState`), capture (state + detail + live fps),
  WLED strip reachability (HTTP probe of the configured host) — polled every 5 s.
- Toggle «Capture» on/off → `capture_set_paused` (SIGUSR1, immediate, runtime only).
- Toggle «Capture at startup» → `capture.start_paused`, saved via `save_config`
  (immediate effect on reload + persisted for boot).
- Sliders Brightness (0–100 %), Gamma (0.5–2.5), Saturation (0–3), Smooth (0.05–1.0):
  every change saves (debounced 400 ms) → live at the daemon's next tick and
  reloaded at boot.
- Button «Configuration» → configuration screen.

## Configuration screen

- Live preview: the 4 segments each light in their own color (bottom red, left blue,
  top green, right yellow), driven by a new `--preview-strip` daemon mode spawned by
  the backend with the draft layout as CLI args; respawned on every draft change;
  killed on exit. **Nothing is written to the config file until «Save and quit»**.
- Fields: WLED IP (`TextField` + green OK / red NOK reachability via HTTP probe,
  probed on change), start point (dropdown, 8 full-word labels, no codes), direction
  (toggle: on = clockwise, off = counterclockwise), LED counts per segment (bottom,
  left, top, right — text fields), offset slider −20…+20, read-only total active LED
  count, band slider 1–20 %, fps text field.
- «Save and quit» → `save_config(draft)` + stop preview + back to main page.
  «Cancel» → stop preview + back to main page, config file untouched.

## Implementation order

1. Rust: `start_paused` in config schema (+ CLI flag), daemon startup/reload behavior,
   `--preview-strip` mode (pattern via `geometry::chain` + `apply_offset`).
2. Backend `main.py`: schema mirror, `probe_wled`, `wled_status`,
   `preview_start`/`preview_update`/`preview_stop` (subprocess management).
3. Frontend `index.tsx`: two views as above.
4. Docs: SPEC §4.2/§4.3/§6.2/§7/§10 + D11, README, plugin README, config example.

**Status:** implemented (see closing commit)

- [x] Rust: `[capture] start_paused` (config.rs/cli.rs `‑‑start-paused`, reload.rs applies the
  pause flag on change, capture.rs `set_paused`, main.rs startup pause) + `--preview-strip`
  mode (preview.rs: bottom red / left blue / top green / right yellow through chain+offset)
- [x] Backend main.py: schema mirror, `probe_wled` / `wled_status` (HTTP /json/info),
  `preview_set` / `preview_stop` (subprocess with PDEATHSIG, draft validated before spawn)
- [x] Frontend: main page (status service/capture/FPS/WLED, Capture + Capture-at-startup
  switches, Brightness/Gamma/Saturation/Smooth sliders, Configuration button) and
  configuration screen (host + OK/NOK, full-word start dropdown, clockwise toggle, 4 segment
  counts + live total, offset −20…+20, band 1–20 %, FPS, Save-and-quit / Cancel; live
  preview via preview_set, auto-pause of the capture on entry, resume on exit)
- [x] Docs: SPEC §2/§4/§6.2/§7/§10/§12, README.md, plugin/README.md, config.toml.example
- [x] cargo test 107+7 green, clippy `-D warnings` clean, cargo fmt clean; plugin
  `npm run build` + `npm run check` (tsc + biome) green; smoke-tested startup pause,
  live reload of start_paused, and --preview-strip against a loopback UDP sink
