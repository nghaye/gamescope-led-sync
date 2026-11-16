# 03: CLI surface + CLI > file > default precedence

**What to build:** The full v1 CLI (SPEC.md §4.4, §14): `--config PATH`, `--dry-run` (never send, log only), one flag per config field (`--wled HOST`, `--port`, `--timeout`, `--top/--right/--bottom/--left`, `--start` with the 8 codes, `--direction <clockwise|counterclockwise>` replacing `--counterclockwise`, `--offset N`, `--fps`, `--sample-height`, `--band`, `--saturation`, `--gamma`, `--smooth`, `--brightness`), plus debug-only `--once`, `--dump PNG`, `--preview`. Merge precedence **CLI flag > config file > built-in default**; validation errors exit with a clap-style error (exit code 2) per §4.3.

**Blocked by:** 02

**Status:** done (commit e6c922a on vibe/ticket-03-cli-717efe)

- [ ] All §4.4 flags exist with correct value parsers; `--counterclockwise` removed in favor of `--direction`
- [ ] Merge precedence CLI > file > default, covered by a precedence unit test
- [ ] Validation errors exit code 2; an explicit `--config` pointing at a missing/unparseable file is fatal at startup (exit code 2)
- [ ] `--dry-run` never sends anything, logs only
- [ ] `cargo test` and `just ci` pass
