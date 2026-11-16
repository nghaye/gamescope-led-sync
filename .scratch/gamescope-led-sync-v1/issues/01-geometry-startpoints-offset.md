# 01: Geometry: 8 start points × direction + offset

**What to build:** Extend the strip geometry so any of the 8 start points (4 corners `bl tl tr br` + 4 edge midpoints `lc tc rc bc`) × direction (`clockwise`/`counterclockwise`, viewed from the front) maps screen zones to physical LEDs, plus the offset feature (SPEC.md D1/D3, §5.2/§5.3): a positive offset sends that many black LEDs first (packet length `L = count + offset`), a negative offset rotates the chain. `chain()` must follow the formal index table of SPEC §5.2; a separate `apply_offset()` builds the final packet.

**Blocked by:** None (can start immediately)

**Status:** done (commit b23442e on vibe/implement-spec-v1-ecb440)

- [ ] `StartPoint` enum with the 8 codes (parse/name round-trip); `Direction` replaces the `clockwise` bool in `Layout`
- [ ] `chain()` matches the SPEC §5.2 index table for all 16 start × direction combos (truth-table unit tests); existing corner tests stay green as the regression baseline
- [ ] `apply_offset()`: black head for `offset > 0`, euclidean rotation for `offset < 0`; `L = count + max(offset,0) <= 490` enforced
- [ ] Midpoint start on a 0-LED edge is a validation error; corner start degenerates gracefully
- [ ] Offset unit tests: 0, +n, −n, `L=490` accepted, `L=491` rejected
- [ ] `cargo test` and `just ci` pass
