# 04: Color grading incl. brightness + --dump/--preview parity

**What to build:** Per-LED grading in the locked order of SPEC.md §5.4: saturation gain → gamma → × brightness → clamp 0..255 → u8, applied on the chain **before** offset padding; all no-ops at defaults (1.0/1.0/1.0). EMA smoothing (`p += smooth·(n−p)` per channel) runs on the chain and resets when `count` changes. The `--dump` / `--preview` diagnostics must grade with saturation + gamma + brightness so they match the LED output exactly (§12).

**Blocked by:** 01, 02

**Status:** done (commit 2ba0b70 on vibe/implement-spec-v1-717efe)

- [ ] Grading pipeline order per §5.4 with `brightness` added; defaults are no-ops
- [ ] Smoothing resets on `count` change (existing length check)
- [ ] `--dump` PNG and `--preview` show colors identical to what is sent to WLED
- [ ] Grading-order unit tests incl. brightness; existing geometry/output tests stay green
- [ ] `cargo test` and `just ci` pass
