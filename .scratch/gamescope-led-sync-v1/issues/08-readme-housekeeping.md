# 08: README rewrite + v1 housekeeping

**What to build:** README rewritten for the SteamOS deliverable (SPEC.md §13): objective (65" TV, CachyOS/gamescope/BC250, WLED), the 4-step pipeline (kept), requirements (Linux, PipeWire user session, `systemd --user`, WLED with UDP realtime DRGB on port 21324, segment length `L = count + max(offset,0)`, ≤ 490 LEDs — §11), install (`install.sh --enable`), configuration reference (TOML table + CLI overrides), service management (`systemctl --user` + Decky), strip geometry (8 start points, direction, offset semantics, WLED segment-length rule), Decky usage, protocol notes (DRGB, 490 cap, DNRGB = future / §15 out of scope), troubleshooting (`journalctl --user -u gamescope-led-sync -f`, `--once --dump`, `--dry-run`), development (`just ci`, tests) and the manual E2E checklist. Housekeeping (§14): `Cargo.toml` version `0.1.0`, remove the dead `../python` reference in the `main.rs` doc comment, drop the «100-inch TV» / «early WIP» wording.

**Blocked by:** None (can start immediately)

**Status:** done (commit c3a15fa on vibe/implement-spec-v1-717efe)

- [ ] README covers all §13 sections and passes the markdownlint `just` recipe
- [ ] Version `0.1.0` in `Cargo.toml`; no `../python` reference left in doc comments
- [ ] `just ci` passes
