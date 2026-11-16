# 06: deploy/ — systemd user unit + config.toml.example + install.sh

**What to build:** The v1 deployment artifacts (SPEC.md §8/§9): `deploy/gamescope-led-sync.service` (user unit: `After=`/`Wants=pipewire.service`, `Restart=on-failure`, `RestartSec=2s`, `WantedBy=default.target`), `deploy/config.toml.example` (fully commented; 65" strip `top=86 right=48 bottom=86 left=48`, `start="bc"`, `direction="clockwise"`, `offset=0`), and `deploy/install.sh` — idempotent: build with `cargo build --release --locked` when cargo is available and no `--binary` given, install the binary to `/usr/bin/gamescope-led-sync` (`sudo install -m 0755`), copy the example config to `~/.config/gamescope-led-sync/config.toml` **only if absent**, install the unit to `~/.config/systemd/user/` + `systemctl --user daemon-reload`, `--enable` → `systemctl --user enable --now gamescope-led-sync`, `--uninstall` → disable --now, remove unit + binary, daemon-reload, and **keep the config file** (documented).

**Blocked by:** 02 (the example must parse against the final schema)

**Status:** ready-for-agent

- [ ] Unit file matches §8 exactly and passes the repo's yamllint recipe
- [ ] `config.toml.example` passes tombi and parses/validates against the 02 schema
- [ ] `install.sh` is idempotent (running twice yields the same state); `--uninstall` keeps the user config
- [ ] `just ci` passes
