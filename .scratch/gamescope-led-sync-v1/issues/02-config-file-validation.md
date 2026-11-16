# 02: Config file (config.rs) + shared validation

**What to build:** TOML config file support with one shared validator (SPEC.md §4): new `config.rs` (new deps: `serde` + `toml` only), full schema §4.2 with bounds and `deny_unknown_fields` (unknown keys are hard errors), `Config::validate()` used by both the CLI and the file, built-in defaults, lookup order §4.1 (`--config` > `$XDG_CONFIG_HOME/gamescope-led-sync/config.toml` > `/etc/gamescope-led-sync/config.toml` > defaults with a warning). No schema-version key in v1.

**Blocked by:** 01 (layout types `StartPoint`/`Direction`, `count >= 1`, `count + max(offset,0) <= 490`)

**Status:** done (commit cf5c7a9 on vibe/implement-spec-v1-ecb440)

- [ ] Schema §4.2 with defaults exactly as spec'd (`[wled]`, `[layout]`, `[processing]`, `[behavior]`, `[capture]` tables)
- [ ] `Config::validate()` enforces every bound, `count >= 1`, packet length `<= 490`, known codes, midpoint-on-empty-edge; errors name the field, the value, and the bound
- [ ] `deny_unknown_fields`: a typo'd key is a hard error
- [ ] Lookup precedence §4.1; an explicit `--config` to a missing/unparseable file is fatal, exit code 2 (CLI wiring lands in 03)
- [ ] Parse/validate unit tests, one per error class
- [ ] `cargo test` and `just ci` pass
