# 13: Preview pattern: on a midpoint start, split the seam edge in two colors

**Reported by the owner (2026-11-21):** in the configuration screen's live
preview (the 4 edges in 4 colors), when the start point is at the **center of
an edge** (e.g. `bc`), paint the two halves of that edge differently — the
part at the **beginning** of the strip and the part at the **end** (left and
right of the center; e.g. red and orange) — so the circular offset is easy
to see on the strip.

**Locked decision:** the strip is a loop, so on a midpoint start the seam
edge's zones are split between the head run and the tail run of the chain.
The head run (beginning of the strip) keeps the edge color; the tail run
(end of the strip, physically next to the beginning) is painted **orange**
`[255,165,0]`. The split is computed from the chain's edge tags, so it follows
the direction automatically. Corner starts are unchanged (no seam edge).
Applies to both `--preview-strip` (bottom red / left blue / top green / right
yellow) and `--setup`'s test pattern (top red / right green / bottom blue /
left yellow). The circular offset rotates the seam colors too, which is what
makes the offset visible.

**Status:** done (commit pending on main)

- [x] `geometry.rs`: `Edge` enum, `seam_edge(start)`, `chain_edges(l)` (same
  ordering as `chain()`), `edge_pattern(l, edge_colors, seam)` = chain tags +
  head/tail split + `apply_offset`; unit tests (seam edge per start point,
  chain order, midpoint split, corner start unsplit)
- [x] `preview.rs` `segment_pattern` and `setup.rs` `test_pattern` use
  `edge_pattern`; preview tests (bc cw/ccw split, odd-edge halves + offset)
- [x] Docs: SPEC.md (§4.4, §10.2, §12), README.md, plugin/README.md,
  `--setup` prompt text, preview log line
- [x] `cargo test` (121 passed), `cargo clippy` clean, no new `cargo fmt` diffs
