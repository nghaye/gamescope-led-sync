# 11: Decky plugin does not see the running service (capture switch greyed out) + capture OFF must release the WLED

**Reported by the owner (2026-11-19), reproduced live on cachyos-bc250:**

1. The plugin's main page shows the service as not running although
   `systemctl --user is-active gamescope-led-sync` says `active`, so the
   «Capture» switch is disabled (`disabled={!capture?.active}`).
2. When the capture is OFF (paused), the service must **release the WLED**;
   with `on_no_session = "keep-last"` the daemon kept resending the last frame
   every second, so the WLED stayed locked in realtime forever.

**Root causes (both confirmed live):**

1. Decky Loader runs as **root** and forks the plugin backend with the root
   environment: `XDG_RUNTIME_DIR` / `DBUS_SESSION_BUS_ADDRESS` are missing, so
   every `systemctl --user` / `journalctl --user` call in the backend fails
   with "Failed to connect to user scope bus" (rc=1, empty stdout) →
   `ActiveState` read as `unknown` → `active: false`. Reproduced through the
   loader WS (`loader/call_plugin_method` → `service_status` returned
   `{"ActiveState":"unknown",...}` while the unit was `active/running`).
2. `wait_and_keepalive` applied `on_no_session` also while manually paused:
   with `keep-last`, one packet per second kept WLED's realtime timeout
   re-armed. A/B-verified with a loopback UDP listener: old binary sent 5
   packets in the 4 s after SIGUSR1, new binary sends 0.

**Fixes:**

- `plugin/main.py`: `_session_env()` fills `XDG_RUNTIME_DIR=/run/user/<uid>`
  and `DBUS_SESSION_BUS_ADDRESS=unix:path=…/bus` when absent, and is passed
  to every `systemctl --user` / `journalctl --user` subprocess; `service_status`
  also returns `error` (the `systemctl` stderr) which the frontend shows in the
  Service field description.
- `src/main.rs`: new pure `no_session_packet(cfg, paused, last_colors)` —
  while paused it returns `None` for **all** `on_no_session` values, so the
  WLED is released (its realtime timeout applies after the last packet's
  `timeout_s`). §6.1 unchanged for the unpaused no-session wait. Pause log
  line now reads «capture paused (SIGUSR1); WLED released, no packets while
  paused» (the `capture paused` prefix the plugin parses is unchanged).

**Status:** done (commit 1dbad3b on main)

- [x] Backend env fix + `error` surfacing; frontend Service field shows it
- [x] Rust: paused ⇒ no packets, unit test `paused_releases_the_wled_regardless_of_on_no_session`
- [x] Verified live through the loader WS: `service_status` →
  `active/running/enabled`, `capture_status` → `capturing` + fps,
  `capture_set_paused` roundtrip pause/resume OK
- [x] Verified live A/B with a loopback UDP listener while the service was
  paused: old binary kept sending after SIGUSR1 (5 packets/4 s), new binary
  sends 0; resume works. `cargo test` 108+7 green, clippy `-D warnings`
  clean, fmt clean on touched files; plugin `npm run build` + `check` green
- [x] Docs: SPEC D5/§6.1/§6.2/§10.1, README.md, plugin/README.md,
  deploy/config.toml.example
- [x] Deploy the new daemon binary: `./deploy/install.sh` (sudo) then
  `systemctl --user restart gamescope-led-sync` — the plugin itself is already
  reinstalled (`./plugin/install.sh --build`) and verified live
