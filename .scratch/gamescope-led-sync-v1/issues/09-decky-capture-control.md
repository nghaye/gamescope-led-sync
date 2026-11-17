# 09: Decky UI — on_no_session setting, capture status, manual capture start/stop

**What to build:** Three additions to the Decky plugin (user request, overrides the
file-only wording of D5/D9 for `on_no_session`):

1. **`behavior.on_no_session` in the UI.** A dropdown in a new «Behavior» section
   of `plugin/src/index.tsx` (preset / keep-last / black, saving through the
   existing debounced `save_config` path — the backend already validates the enum
   in `main.py`). Remove the «file-only» wording from SPEC.md §2/§10.2, the plugin
   README, `deploy/config.toml.example`, and the main README; `wled.timeout_s`
   stays file-only.
2. **Capture status in the UI.** Backend `capture_status()`: service
   ActiveState plus the capture state parsed from the last ~200 journal lines
   («capturing» after the last «found gamescope node», «waiting» after
   «session ended»/«retrying»/«waiting for the gamescope node», «paused» after
   «capture paused», plus the live fps from the last stats line). Frontend: a
   read-only Field in the Service section, refreshed on mount, after every
   action, and on a 5 s interval.
3. **Manual capture start/stop.** The daemon learns SIGUSR1 = toggle pause
   (`src/capture.rs` + `src/main.rs`): while paused it leaves the reconnect
   wait loop, so `on_no_session` applies and live reload keeps working; the
   capture stream disconnects cleanly on pause and reconnects on resume. The
   daemon logs «capture paused (SIGUSR1)… » / «capture resumed (SIGUSR1)» on
   each transition (that is what `capture_status` parses). Backend
   `capture_set_paused(bool)`: refuse when the service is not active, compare
   with the current parsed state, then `systemctl --user kill -s SIGUSR1
   gamescope-led-sync` only when a toggle is needed. Frontend: «Stop capture» /
   «Start capture» buttons next to the capture status.

**Status:** implemented, uncommitted (no commit requested this session)

- [x] Daemon: SIGUSR1 pause/resume, `Reason::Paused`, clean stream teardown on pause, transition logs; `cargo test` 100+7 green, clippy `-D warnings` clean
- [x] Backend: `capture_status()` + `capture_set_paused(bool)` in `plugin/main.py` (scratch-tested against simulated journal output)
- [x] Frontend: Behavior dropdown for `on_no_session`, capture status field + 5 s refresh, Stop/Start capture buttons; `npm run build` + `npm run check` + biome format/lint green
- [x] Docs updated (SPEC §2/§6.2/§10, plugin README, main README, config.toml.example)
