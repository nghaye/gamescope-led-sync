# 12: Offset becomes a circular shift of the whole strip (loop), replacing the black-head padding

**Reported by the owner (2026-11-21):** the offset setting is buggy — a
**negative offset crashes the WLED stream** — and the semantics should change:
instead of carving a black gap, the offset should **shift the whole LED strip
left or right**. Treat the strip as a **loop**: the last LED is physically
next to the first.

**Locked decision (revises SPEC D1):** `offset` is a circular shift of the
whole chain, whatever its sign: `packet[i] = chain[(i - offset) mod count]`.
`offset > 0` shifts toward higher LED indices (right along the strip),
`offset < 0` toward lower indices (left), wrapping around the loop. The DRGB
packet length is **always `count`** (≤ 490) — the offset never changes it, so
a sign change can no longer break the WLED realtime stream (the old
`count + max(offset,0)` packet-length rule is gone, along with the
`PacketTooLong` error and the leading-black-LEDs padding).

**Status:** done (commit b064d94 on main)

- [x] `geometry.rs`: `apply_offset()` = euclidean rotation for any sign, `packet_len() = count()`, `Layout::new` only checks `count ≤ 490`, `PacketTooLong` removed; unit tests updated (0, +n, −n, `|offset| > count` wraps, 490 ok / 491 rejected)
- [x] `cli.rs` `--offset` help text; `config.rs` / `preview.rs` / `setup.rs` / `capture.rs` comments + tests
- [x] `plugin/main.py`: validation drops the `count + max(offset,0) ≤ 490` rule; `preview_set` returns `packet_len = count`
- [x] `plugin/src/index.tsx`: offset slider description (circular shift); dist rebuilt
- [x] Docs: SPEC.md (D1, §4.2/§4.3, §5.3/§5.4, §11, §12, audit trail), README.md, plugin/README.md, deploy/config.toml.example
- [x] `cargo test` (116 passed), `cargo clippy`, plugin `tsc` + `biome` + rollup build
