# 07: Decky Loader plugin (plugin/)

**What to build:** The Decky plugin (SPEC.md §10): scaffold `plugin/` from the official Decky template (rollup + `decky-frontend-lib` + `@decky/api`), `plugin.json` name «Gamescope LED Sync», version 0.1.0, **no `root` flag** (D6), English UI (D7). Backend `main.py` with the §10.1 methods: `get_config()` (parse TOML with `tomllib`, defaults merged), `save_config(cfg)` (Python mirror of the §4.3 validation; write **atomically** tmp + `rename`; return `{ok, errors}`; the daemon applies it at the next tick), `service_status()` / `service_set_enabled()` / `service_start()` / `service_stop()` / `service_restart()` via `systemctl --user`, `get_logs(n)` via `journalctl --user`, and `test_frame()` — refuses while the service is active (two capture clients on one gamescope node are unsupported), otherwise runs the binary with `--config <cfg> --once --dump <plugin data dir>/last-frame.png` (30 s timeout, clear error if the binary is missing), PNG served at `/plugins/<name>/data/last-frame.png`. Frontend `src/index.tsx` per §10.2: service section (enabled-at-boot toggle, status line, Start/Stop/Restart, logs modal, test-frame modal with the PNG), WLED host + port, four strip counts, start (8 labelled options) and direction dropdowns, offset slider **−30..+30 step 1** (D11), image sliders saturation/gamma/brightness/band/smooth plus numeric fps and «Advanced» sample_height (D9 ranges), every control saving debounced 400 ms with a «Saved ✓» / errors status line (no Apply button), and the packet-length hint `L = count + max(offset,0)` highlighted whenever L changes. `wled.timeout_s` and `behavior.on_no_session` stay file-only (D9/D5), documented in the plugin README.

**Blocked by:** 02, 03 (`test_frame` relies on `--config` / `--once` / `--dump`)

**Status:** ready-for-agent

- [ ] Backend methods match the §10.1 table; `save_config` is atomic + validated; the `test_frame` guard works
- [ ] Frontend controls match the §10.2 mapping incl. D9 slider ranges and the D11 offset range
- [ ] 400 ms debounce on save; status line; L hint highlighted on change
- [ ] Plugin builds with its template toolchain (rollup/biome); repo `just ci` passes
