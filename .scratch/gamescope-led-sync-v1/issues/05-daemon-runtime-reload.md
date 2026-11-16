# 05: Daemon runtime: shared config, non-fatal sender, live reload, no-session behavior

**What to build:** Rework `main.rs`/`capture.rs` so the config is shared (`Rc<RefCell<Config>>`) instead of cloned into the timer closure (SPEC.md §6.0, §7, §6.1/D5). `WledSender` creation failure must never exit: log a warning, run senderless (like `--dry-run`), retry every 5 s and on every config reload; an unresolvable host after a reload keeps the old sender. Every tick, `stat()` the config file (mtime + len) and on change: valid → atomically swap the shared config (recreate the sender if `wled.host`/`port` changed, reset smoothing if `count` changed, update the session timer interval if `fps` changed, `info!`-log each changed field); invalid → `warn!` and keep the previous config; deleted/unreadable → keep the current config with one debug log on state change. While disconnected, `on_no_session = "preset"` (default) sends **nothing** (WLED falls back to its preset after `timeout_s`), `keep-last` resends `last_colors` every 1 s, `black` sends black every 1 s; if no frame was ever captured, all three send nothing until the first frame. Mid-session 1 s keepalive, 5 s stats log, reconnect loop, signal handling, `media.name` discovery and `requested_size` negotiation stay unchanged. On SIGTERM the daemon sends nothing.

**Blocked by:** 02, 03, 04

**Status:** done (commit d0d4a6d on vibe/implement-spec-v1-717efe)

- [ ] No exit when `WledSender::new` fails; senderless retry every 5 s and on reload
- [ ] Per-tick stat-based reload with atomic swap and all per-field effects of §7
- [ ] Invalid or deleted config at runtime keeps the previous config (`warn!` / debug-once, no spam)
- [ ] `on_no_session` keep-last / black / preset per §6.1; mid-session keepalive unchanged
- [ ] Reload unit tests: valid swap applies, invalid keeps old
- [ ] `cargo test` and `just ci` pass
