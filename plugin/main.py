"""Decky Loader backend for gamescope-led-sync (SPEC §10.1).

Runs as the session user (no root flag, D6), so `systemctl --user` and
writing ~/.config/... work directly. The daemon reads its config file and
applies changes live at the next tick (≤ 1/fps), so save_config only has
to validate and write the file atomically (temp file + rename).
"""

import os
import subprocess
import tempfile
import tomllib
from pathlib import Path

CONFIG = Path.home() / ".config" / "gamescope-led-sync" / "config.toml"
BINARY = "/usr/bin/gamescope-led-sync"
UNIT = "gamescope-led-sync"

# Plugin data dir, from the loader's DECKY_* env vars. The loader serves
# /plugins/<plugin name>/data/... from DECKY_PLUGIN_RUNTIME_DIR
# (~/homebrew/data/<plugin dir>), so the PNG written by test_frame lands there.
DATA_DIR = Path(
    os.environ.get("DECKY_PLUGIN_RUNTIME_DIR")
    or os.environ.get("DECKY_PLUGIN_DATA_DIR")
    or (Path(os.environ["DECKY_PLUGIN_DIR"]) / "data" if os.environ.get("DECKY_PLUGIN_DIR") else Path.home() / ".decky-data")
)

# Built-in defaults, mirroring Config::defaults() (SPEC §4.2).
DEFAULTS = {
    "wled": {"host": None, "port": 21324, "timeout_s": 2},
    "layout": {
        "top": 132,
        "right": 75,
        "bottom": 132,
        "left": 75,
        "start": "bl",
        "direction": "clockwise",
        "offset": 0,
    },
    "processing": {
        "fps": 30.0,
        "sample_height": 240,
        "band_frac": 0.08,
        "saturation": 1.0,
        "gamma": 1.0,
        "smooth": 0.5,
        "brightness": 1.0,
    },
    "behavior": {"on_no_session": "preset"},
    "capture": {"node_name": "gamescope"},
}

# The file schema (SPEC §4.2), in file order: unknown keys are a hard
# error, like the daemon's deny_unknown_fields.
SCHEMA = {
    "wled": ["host", "port", "timeout_s"],
    "layout": ["top", "right", "bottom", "left", "start", "direction", "offset"],
    "processing": ["fps", "sample_height", "band_frac", "saturation", "gamma", "smooth", "brightness"],
    "behavior": ["on_no_session"],
    "capture": ["node_name"],
}

START_CODES = ["bl", "bc", "br", "rc", "tr", "tc", "tl", "lc"]
DIRECTIONS = ["clockwise", "counterclockwise"]
ON_NO_SESSION = ["keep-last", "black", "preset"]


def _err(errors, field, value, bound):
    errors.append(f"{field}: {value!r} violates {bound}")


def _num(errors, section, key, value, lo, hi, integer=False, lo_open=False, hi_open=False):
    """Validate one numeric field against its §4.2 bound."""
    field = f"{section}.{key}"
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        _err(errors, field, value, "a number")
        return
    if integer and not isinstance(value, int):
        _err(errors, field, value, "an integer")
        return
    if lo is not None and (value <= lo if lo_open else value < lo):
        _err(errors, field, value, f">{lo}" if lo_open else f">= {lo}")
        return
    if hi is not None and (value >= hi if hi_open else value > hi):
        _err(errors, field, value, f"<{hi}" if hi_open else f"<= {hi}")


def validate(cfg):
    """Python mirror of the daemon's Config::validate() (SPEC §4.3).

    Returns a list of error strings naming the field, the value, and the
    bound; empty means the config is valid.
    """
    errors = []
    for section, keys in SCHEMA.items():
        if section not in cfg or not isinstance(cfg[section], dict):
            _err(errors, section, cfg.get(section), "a table")
            continue
        for key in cfg[section]:
            if key not in keys:
                _err(errors, f"{section}.{key}", cfg[section][key], "a known key")

    wled = cfg.get("wled", {})
    host = wled.get("host")
    if host is not None and (not isinstance(host, str) or not host):
        _err(errors, "wled.host", host, "non-empty hostname or IP")
    _num(errors, "wled", "port", wled.get("port"), 1, 65535, integer=True)
    _num(errors, "wled", "timeout_s", wled.get("timeout_s"), 1, 255, integer=True)

    layout = cfg.get("layout", {})
    for edge in ("top", "right", "bottom", "left"):
        _num(errors, "layout", edge, layout.get(edge), 0, 490, integer=True)
    start = layout.get("start")
    if start is not None and start not in START_CODES:
        _err(errors, "layout.start", start, "one of " + " ".join(START_CODES))
    direction = layout.get("direction")
    if direction is not None and direction not in DIRECTIONS:
        _err(errors, "layout.direction", direction, "clockwise|counterclockwise")
    _num(errors, "layout", "offset", layout.get("offset"), None, None, integer=True)

    processing = cfg.get("processing", {})
    _num(errors, "processing", "fps", processing.get("fps"), 1, 240)
    _num(errors, "processing", "sample_height", processing.get("sample_height"), 1, 16384, integer=True)
    _num(errors, "processing", "band_frac", processing.get("band_frac"), 0, 1, lo_open=True, hi_open=True)
    _num(errors, "processing", "saturation", processing.get("saturation"), 0, None)
    _num(errors, "processing", "gamma", processing.get("gamma"), 0, None, lo_open=True)
    _num(errors, "processing", "smooth", processing.get("smooth"), 0, None, lo_open=True)
    _num(errors, "processing", "brightness", processing.get("brightness"), 0, 1)

    behavior = cfg.get("behavior", {})
    on_no_session = behavior.get("on_no_session")
    if on_no_session is not None and on_no_session not in ON_NO_SESSION:
        _err(errors, "behavior.on_no_session", on_no_session, "keep-last|black|preset")

    capture = cfg.get("capture", {})
    node_name = capture.get("node_name")
    if node_name is not None and (not isinstance(node_name, str) or not node_name):
        _err(errors, "capture.node_name", node_name, "non-empty")

    # Layout rules (SPEC §4.3/§5.2): count >= 1, packet length <= 490,
    # and a midpoint start needs its edge to have at least 1 LED.
    edges = {e: layout.get(e, DEFAULTS["layout"][e]) for e in ("top", "right", "bottom", "left")}
    if all(isinstance(v, int) for v in edges.values()):
        count = sum(edges.values())
        if count < 1:
            _err(errors, "layout", count, "count >= 1")
        elif count > 490:
            _err(errors, "layout", count, "<= 490 (one DRGB packet)")
        offset = layout.get("offset", DEFAULTS["layout"]["offset"])
        if isinstance(offset, int) and count + max(offset, 0) > 490:
            _err(errors, "layout.offset", offset, f"packet length {count + max(offset, 0)} <= 490")
        start = layout.get("start", DEFAULTS["layout"]["start"])
        midpoint_edge = {"lc": "left", "tc": "top", "rc": "right", "bc": "bottom"}.get(start)
        if midpoint_edge and edges[midpoint_edge] < 1:
            _err(errors, "layout.start", start, f"requires {midpoint_edge} >= 1")

    return errors


def _deep_merge(base, over):
    out = dict(base)
    for key, value in over.items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = _deep_merge(out[key], value)
        else:
            out[key] = value
    return out


def _systemctl(*args):
    return subprocess.run(
        ["systemctl", "--user", *args],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )


class Plugin:
    """Backend methods called from the frontend via callBackend (SPEC §10.1)."""

    async def get_config(self):
        """Parse the TOML config, defaults merged (tomllib)."""
        try:
            with open(CONFIG, "rb") as f:
                file_cfg = tomllib.load(f)
        except FileNotFoundError:
            file_cfg = {}
        except tomllib.TOMLDecodeError as e:
            return {"ok": False, "error": f"cannot parse {CONFIG}: {e}", "config": DEFAULTS}
        return {"ok": True, "config": _deep_merge(DEFAULTS, file_cfg)}

    async def save_config(self, cfg):
        """Validate (Python mirror of §4.3) and write atomically (tmp + rename).

        The daemon applies the new config at the next tick (SPEC §7); an
        invalid config is not written.
        """
        merged = _deep_merge(DEFAULTS, cfg)
        errors = validate(merged)
        if errors:
            return {"ok": False, "errors": errors}
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=CONFIG.parent, prefix=".config-", suffix=".toml")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                for section, keys in SCHEMA.items():
                    f.write(f"[{section}]\n")
                    for key in keys:
                        value = merged[section][key]
                        if value is None:
                            continue
                        if isinstance(value, str):
                            f.write(f'{key} = "{value}"\n')
                        elif isinstance(value, bool):
                            f.write(f"{key} = {str(value).lower()}\n")
                        else:
                            f.write(f"{key} = {value}\n")
                    f.write("\n")
            os.replace(tmp, CONFIG)
        except BaseException:
            os.unlink(tmp)
            raise
        return {"ok": True}

    async def service_status(self):
        show = _systemctl("show", UNIT, "-p", "ActiveState,SubState,UnitFileState")
        props = dict(line.split("=", 1) for line in show.stdout.splitlines() if "=" in line)
        is_enabled = _systemctl("is-enabled", UNIT)
        return {
            "ok": True,
            "ActiveState": props.get("ActiveState", "unknown"),
            "SubState": props.get("SubState", ""),
            "UnitFileState": props.get("UnitFileState", ""),
            "enabled": is_enabled.returncode == 0,
        }

    async def service_set_enabled(self, enabled: bool):
        action = "enable" if enabled else "disable"
        res = _systemctl(action, UNIT)
        return {"ok": res.returncode == 0, "error": res.stderr.strip() or None}

    async def service_start(self):
        res = _systemctl("start", UNIT)
        return {"ok": res.returncode == 0, "error": res.stderr.strip() or None}

    async def service_stop(self):
        res = _systemctl("stop", UNIT)
        return {"ok": res.returncode == 0, "error": res.stderr.strip() or None}

    async def service_restart(self):
        res = _systemctl("restart", UNIT)
        return {"ok": res.returncode == 0, "error": res.stderr.strip() or None}

    async def get_logs(self, n: int = 200):
        res = subprocess.run(
            ["journalctl", "--user", "-u", UNIT, "-n", str(n), "--no-pager"],
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        return {"ok": res.returncode == 0, "logs": res.stdout + res.stderr}

    async def test_frame(self):
        """Run the binary once with --once --dump (SPEC §10.1, D10).

        Refused while the service is active: two capture clients on one
        gamescope node are unsupported (SPEC §6.0).
        """
        status = await self.service_status()
        if status.get("ActiveState") == "active":
            return {"ok": False, "error": "stop the service first"}
        if not os.path.exists(BINARY):
            return {"ok": False, "error": f"binary not found: {BINARY} (run install.sh first)"}
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        png = DATA_DIR / "last-frame.png"
        try:
            proc = subprocess.run(
                [BINARY, "--config", str(CONFIG), "--once", "--dump", str(png)],
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return {"ok": False, "error": "timed out after 30 s"}
        log = (proc.stdout + proc.stderr).strip()
        if proc.returncode != 0:
            return {"ok": False, "log": log, "error": f"exit code {proc.returncode}"}
        return {"ok": png.exists(), "log": log}
