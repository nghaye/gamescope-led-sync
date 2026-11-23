# 14: `gap` — a hole at the loop seam (between the strip's last and first LED)

**Reported by the owner (2026-11-22):** the owner's strip starts at
**bottom-center** (`start="bc"`) and runs **clockwise** (to the left), goes
around the TV, and ends at the bottom **right** — 37 LEDs on the left part
(LEDs 1–37), 36 on the right part (the last LED is the strip's last). Between
the left and right halves of the bottom there is a **creux** the width of a
cable duct (**≈ 7 LED pitches**), i.e. a hole between the **1st and the last
LED** of the strip. The WLED segment holds only the real LEDs
(`count = 37 + 36`), the creux is not part of it.

**Locked decision (new D13):** `gap` (0..=50, default 0) is the hole at the
loop seam, in LED pitches, between the strip's last and first LED. The packet
length **stays `count`** (the WLED segment is unchanged — the creux has no
LEDs). The hole lies on the **seam edge** (the start edge for a midpoint start,
at the corner for a corner start): that edge is divided into `n + gap` equal
zones and the `gap` zones at the seam boundary are **dropped**, so no LED
samples the screen region behind the creux and each LED shows the screen zone
physically in front of it. `chain()` / `apply_offset()` are unchanged — the
offset still rotates the `count`-LED chain, and wrapping across the seam
naturally jumps the hole (the seam edge's kept zones sit at the creux edges).
UI: a **slider in the configuration screen, 0..50, step 1, default 0**.
Config key `[layout] gap`, CLI flag `--gap N`.

**Status:** done (commit <hash> on main)

- [x] `geometry.rs`: `Layout.gap` + `with_gap()` + `MAX_GAP`/`GapTooLarge`,
  `seam_hole()`, hole-aware zone sampling in `edge_means()` (`zones_x_hole` /
  `zones_y_hole`); unit tests (bound 50 ok / 51 rejected, seam-hole position
  per start/direction, bottom-edge sampling skips the creux blocks, chain seam
  sits at the creux edges, packet length stays `count`)
- [x] `config.rs`: `[layout] gap` key, merge, validate via `with_gap`; tests
  (full file, bound 51 → `GapTooLarge`, default 0)
- [x] `cli.rs`: `--gap N` flag, merge precedence CLI > file > default; tests
- [x] `plugin/main.py`: DEFAULTS, SCHEMA, `validate()` (0..=50 integer),
  `_layout_args` passes `--gap` to the preview process
- [x] `plugin/src/index.tsx`: `Cfg.layout.gap`, DEFAULTS, gap slider **0..50
  step 1** in the configuration screen (LED strip section, after offset);
  dist rebuilt
- [x] Docs: SPEC.md (D13, §4.2/§4.3/§4.4, §5.1/§5.3, §10.1/§10.2, §11, §12),
  README.md, plugin/README.md, deploy/config.toml.example, tests/cli.rs
  (`--help` lists `--gap`, `--gap 51` exits 2)
- [x] `cargo test` (128 passed), `cargo clippy` clean, `cargo fmt` clean on the
  touched files, plugin `tsc` + `biome` + rollup build
