"""Decky Loader backend for gamescope-led-sync (SPEC §10.1).

Runs as the session user (no root flag, D6), so `systemctl --user` and
writing ~/.config/... work directly. The daemon reads its config file and
applies changes live at the next tick (≤ 1/fps), so save_config only has
to validate and write the file atomically (temp file + rename).
"""

import http.client
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import tomllib
from pathlib import Path

CONFIG = Path.home() / ".config" / "gamescope-led-sync" / "config.toml"
BINARY = str(Path.home() / ".local" / "bin" / "gamescope-led-sync")
UNIT = "gamescope-led-sync"

# The plugin's own install dir: the loader sets DECKY_PLUGIN_DIR to
# ~/homebrew/plugins/gamescope-led-sync. The distributed zip ships the payload
# it installs from under payload/ (SPEC §11).
PLUGIN_DIR = Path(os.environ.get("DECKY_PLUGIN_DIR") or Path(__file__).resolve().parent)
PAYLOAD = PLUGIN_DIR / "payload"
PAYLOAD_BIN = PAYLOAD / "bin" / "gamescope-led-sync"
PAYLOAD_UNIT = PAYLOAD / "systemd" / "gamescope-led-sync.service"
PAYLOAD_CONFIG = PAYLOAD / "config" / "config.toml.example"
PAYLOAD_DESKTOP = PAYLOAD / "desktop" / "gamescope-led-sync-calibrate.desktop"
PAYLOAD_LICENSE = PAYLOAD / "LICENSE"

# Install targets — all user-space, no sudo (D6).
BIN_DIR = Path.home() / ".local" / "bin"
UNIT_DIR = Path.home() / ".config" / "systemd" / "user"
UNIT_FILE = UNIT_DIR / "gamescope-led-sync.service"
LICENSE_DEST = Path.home() / ".local" / "share" / "gamescope-led-sync" / "LICENSE"
# The calibration launcher, installed next to the apps so it can be added as
# a non-Steam game in Steam Big Picture (ticket 18): the payload .desktop
# carries a @BIN@ placeholder filled with the installed binary's path.
DESKTOP_DEST = Path.home() / ".local" / "share" / "applications" / "gamescope-led-sync-calibrate.desktop"

# Serializes install/update/uninstall so two taps on a button cannot race.
_install_lock = threading.Lock()

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
        "gap": 0,
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
    "capture": {"node_name": "gamescope", "start_paused": False},
}

# The file schema (SPEC §4.2), in file order: unknown keys are a hard
# error, like the daemon's deny_unknown_fields.
SCHEMA = {
    "wled": ["host", "port", "timeout_s"],
    "layout": ["top", "right", "bottom", "left", "start", "direction", "offset", "gap"],
    "processing": ["fps", "sample_height", "band_frac", "saturation", "gamma", "smooth", "brightness"],
    "behavior": ["on_no_session"],
    "capture": ["node_name", "start_paused"],
}

START_CODES = ["bl", "bc", "br", "rc", "tr", "tc", "tl", "lc"]
DIRECTIONS = ["clockwise", "counterclockwise"]
ON_NO_SESSION = ["keep-last", "black", "preset"]

# WLED's HTTP port, probed with GET /json/info (same probe as --setup).
WLED_HTTP_PORT = 80
PROBE_TIMEOUT_S = 3.0

# The running --preview-strip subprocess of the configuration screen
# (ticket 10): the plugin spawns it with the draft layout as CLI flags
# and respawns it on every draft change; preview_stop kills it.
_preview_proc = None


def _pdeathsig():
    """preexec_fn: die with the plugin backend if the loader goes away,
    so a preview process never lingers on the strip (Linux prctl)."""
    try:
        import ctypes

        ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)  # PR_SET_PDEATHSIG
    except Exception:
        pass


def _probe_wled(host, timeout=PROBE_TIMEOUT_S):
    """GET /json/info on the WLED controller (port 80). Returns
    (info, None) when reachable, (None, error) otherwise."""
    try:
        addrs = socket.getaddrinfo(host, WLED_HTTP_PORT, proto=socket.IPPROTO_TCP)
    except socket.gaierror as e:
        return None, f"cannot resolve {host}: {e}"
    last = None
    for family, _stype, _proto, _canon, addr in addrs:
        try:
            conn = http.client.HTTPConnection(addr[0], WLED_HTTP_PORT, timeout=timeout)
            conn.request("GET", "/json/info", headers={"Host": host, "Connection": "close"})
            resp = conn.getresponse()
            body = resp.read().decode("utf-8", "replace")
            conn.close()
            if resp.status != 200:
                return None, f"{host}: HTTP {resp.status} on /json/info"
            ver = None
            try:
                ver = json.loads(body).get("ver")
            except ValueError:
                pass
            return "HTTP 200 on /json/info" + (f" (WLED {ver})" if ver else ""), None
        except OSError as e:
            last = f"{addr[0]}: {e}"
    return None, f"cannot connect to {host}:{WLED_HTTP_PORT}: {last}"


def _layout_args(layout):
    """The draft layout as CLI flags overriding the config file (§4.1)."""
    return [
        "--top", str(int(layout["top"])),
        "--right", str(int(layout["right"])),
        "--bottom", str(int(layout["bottom"])),
        "--left", str(int(layout["left"])),
        "--start", str(layout["start"]),
        "--direction", str(layout["direction"]),
        "--offset", str(int(layout["offset"])),
        "--gap", str(int(layout["gap"])),
    ]


def _stop_preview():
    global _preview_proc
    if _preview_proc is not None:
        try:
            _preview_proc.terminate()
            _preview_proc.wait(timeout=5)
        except Exception:
            try:
                _preview_proc.kill()
            except Exception:
                pass
        _preview_proc = None


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
    _num(errors, "layout", "gap", layout.get("gap"), 0, 50, integer=True)

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
    start_paused = capture.get("start_paused")
    if start_paused is not None and not isinstance(start_paused, bool):
        _err(errors, "capture.start_paused", start_paused, "a boolean")

    # Layout rules (SPEC §4.3/§5.2): count >= 1 and <= 490 (one DRGB
    # packet; the offset is a circular shift of the loop and the gap is
    # a hole without LEDs, so neither lengthens the packet), and a
    # midpoint start needs its edge to have at least 1 LED.
    edges = {e: layout.get(e, DEFAULTS["layout"][e]) for e in ("top", "right", "bottom", "left")}
    if all(isinstance(v, int) for v in edges.values()):
        count = sum(edges.values())
        if count < 1:
            _err(errors, "layout", count, "count >= 1")
        elif count > 490:
            _err(errors, "layout", count, "<= 490 (one DRGB packet)")
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


_SESSION_ENV = None


def _session_env():
    """Environment for `systemctl --user` / `journalctl --user`.

    The Decky loader runs as root and forks this backend with the root
    environment, so XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS are
    usually missing and every `--user` call fails with "Failed to connect
    to user scope bus" — the plugin would then report the service as
    unknown even while it runs. Point them at the session user's runtime
    dir when they are absent.
    """
    global _SESSION_ENV
    if _SESSION_ENV is None:
        env = dict(os.environ)
        if not env.get("XDG_RUNTIME_DIR"):
            rt = f"/run/user/{os.getuid()}"
            if os.path.isdir(rt):
                env["XDG_RUNTIME_DIR"] = rt
        rt = env.get("XDG_RUNTIME_DIR")
        bus = os.path.join(rt, "bus") if rt else None
        if bus and not env.get("DBUS_SESSION_BUS_ADDRESS") and os.path.exists(bus):
            env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={bus}"
        _SESSION_ENV = env
    return _SESSION_ENV


def _systemctl(*args):
    return subprocess.run(
        ["systemctl", "--user", *args],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
        env=_session_env(),
    )


def _journalctl(*args):
    return subprocess.run(
        ["journalctl", "--user", "-u", UNIT, *args, "--no-pager"],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
        env=_session_env(),
    )


_VERSION_RE = re.compile(r"(\d+\.\d+\.\d+)")


def _plugin_version():
    """This plugin's own version, read from plugin.json next to main.py
    (SPEC §11, D14): the same number Cargo.toml carried at packaging time."""
    try:
        with open(PLUGIN_DIR / "plugin.json", encoding="utf-8") as f:
            return str(json.load(f).get("version") or "unknown")
    except (OSError, ValueError):
        return "unknown"


def _binary_version():
    """The installed binary's version, parsed from `<binary> --version` (one
    line `gamescope-led-sync X.Y.Z`). None when the binary is missing, not
    executable, or does not answer — the install is then treated as broken
    and «Mettre à jour» is offered."""
    if not (os.path.isfile(BINARY) and os.access(BINARY, os.X_OK)):
        return None
    try:
        res = subprocess.run(
            [BINARY, "--version"], capture_output=True, text=True, timeout=5, check=False
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if res.returncode != 0:
        return None
    m = _VERSION_RE.search(res.stdout)
    return m.group(1) if m else None


def _install_desktop(steps):
    """Install the calibration .desktop into ~/.local/share/applications with
    @BIN@ replaced by the installed binary (ticket 18)."""
    if not PAYLOAD_DESKTOP.is_file():
        return False
    DESKTOP_DEST.parent.mkdir(parents=True, exist_ok=True)
    text = PAYLOAD_DESKTOP.read_text(encoding="utf-8").replace("@BIN@", BINARY)
    fd, tmp = tempfile.mkstemp(dir=DESKTOP_DEST.parent, prefix=".desktop-", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
        os.chmod(tmp, 0o644)
        os.replace(tmp, DESKTOP_DEST)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    steps.append(f"calibration .desktop -> {DESKTOP_DEST}")
    return True


def _copy_atomic(src, dst, mode):
    """Copy src to dst atomically: temp file in dst's directory, chmod, then
    rename. Overwriting a running binary in place would fail with «text file
    busy»; a rename never does."""
    dst = Path(dst)
    dst.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=dst.parent, prefix=f".{dst.name}-", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as out, open(src, "rb") as inp:
            shutil.copyfileobj(inp, out)
        os.chmod(tmp, mode)
        os.replace(tmp, dst)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


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
            # Surfaced in the UI: a failed `systemctl --user show` (empty
            # stdout) is what makes the service look "unknown".
            "error": show.stderr.strip() or None if show.returncode != 0 else None,
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

    async def install_status(self):
        """The whole installation state, for the Installation section of the
        main page (SPEC §11): this plugin's version, whether the embedded
        payload is present (a dev install has none), what is installed on the
        machine (binary + unit + config, and the binary's --version), the
        service's run/enabled state, and whether the installed version matches
        the plugin's — a mismatch in either direction means «Mettre à jour»."""
        status = await self.service_status()
        plugin_version = _plugin_version()
        binary_present = os.path.isfile(BINARY) and os.access(BINARY, os.X_OK)
        binary_version = _binary_version()
        versions_match = binary_version == plugin_version if binary_present else None
        return {
            "ok": True,
            "plugin_version": plugin_version,
            "payload_present": PAYLOAD_BIN.is_file(),
            "payload_desktop_present": PAYLOAD_DESKTOP.is_file(),
            "desktop_installed": DESKTOP_DEST.is_file(),
            "binary_installed": binary_present,
            "binary_version": binary_version,
            "unit_installed": UNIT_FILE.is_file(),
            "config_present": CONFIG.is_file(),
            "ActiveState": status.get("ActiveState", "unknown"),
            "SubState": status.get("SubState", ""),
            "active": status.get("ActiveState") == "active",
            "enabled": bool(status.get("enabled")),
            "versions_match": versions_match,
            "update_needed": bool(binary_present and versions_match is False),
            "error": status.get("error"),
        }

    async def service_install(self):
        """Install the service + binary from the embedded payload (SPEC §11):
        binary → ~/.local/bin (atomically, 0755), unit →
        ~/.config/systemd/user (0644), example config →
        ~/.config/gamescope-led-sync/config.toml only when absent (a user
        config is never overwritten), LICENSE → ~/.local/share/…, then
        daemon-reload and enable --now (install = enabled at boot + running).
        Idempotent; refuses to run concurrently with another install."""
        if not _install_lock.acquire(blocking=False):
            return {"ok": False, "error": "an install/update is already running"}
        try:
            if not PAYLOAD_BIN.is_file():
                return {
                    "ok": False,
                    "error": "no embedded payload — the plugin was installed in dev mode; from a checkout run: just install-all",
                }
            steps = []
            _copy_atomic(PAYLOAD_BIN, BINARY, 0o755)
            steps.append(f"binary → {BINARY}")
            _copy_atomic(PAYLOAD_UNIT, UNIT_FILE, 0o644)
            steps.append(f"unit → {UNIT_FILE}")
            if not CONFIG.exists():
                _copy_atomic(PAYLOAD_CONFIG, CONFIG, 0o644)
                steps.append(f"example config → {CONFIG}")
            if PAYLOAD_LICENSE.is_file():
                _copy_atomic(PAYLOAD_LICENSE, LICENSE_DEST, 0o644)
                steps.append(f"license → {LICENSE_DEST}")
            _install_desktop(steps)
            r = _systemctl("daemon-reload")
            if r.returncode != 0:
                return {"ok": False, "error": f"daemon-reload failed: {r.stderr.strip()}", "steps": steps}
            steps.append("systemctl --user daemon-reload")
            r = _systemctl("enable", "--now", UNIT)
            if r.returncode != 0:
                return {"ok": False, "error": f"enable --now failed: {r.stderr.strip()}", "steps": steps}
            steps.append("systemctl --user enable --now")
            return {"ok": True, "version": _plugin_version(), "steps": steps}
        finally:
            _install_lock.release()

    async def service_uninstall(self):
        """Uninstall the service + binary (SPEC §11): disable --now (best
        effort), remove the unit and the binary (plus any leftover .bak), then
        daemon-reload. The config file is kept."""
        if not _install_lock.acquire(blocking=False):
            return {"ok": False, "error": "an install/update is already running"}
        try:
            _systemctl("disable", "--now", UNIT)
            steps = ["systemctl --user disable --now (best effort)"]
            for p in (UNIT_FILE, Path(BINARY), Path(f"{BINARY}.bak"), DESKTOP_DEST):
                if p.exists():
                    p.unlink()
                    steps.append(f"removed {p}")
            r = _systemctl("daemon-reload")
            if r.returncode != 0:
                return {"ok": False, "error": f"daemon-reload failed: {r.stderr.strip()}", "steps": steps}
            steps.append("systemctl --user daemon-reload")
            return {
                "ok": True,
                "steps": steps,
                "config_kept": str(CONFIG) if CONFIG.exists() else None,
            }
        finally:
            _install_lock.release()

    async def service_update(self):
        """Update the installed service + binary to this plugin's embedded
        payload (SPEC §11): stop the service when active, back the old binary
        up to .bak, replace binary + unit atomically, daemon-reload, then
        re-enable / restart exactly as before. When a step fails after the
        replacement, the .bak is restored (rollback) and kept for inspection;
        on success the .bak is removed. A version mismatch in either direction
        is fixed the same way (downgrade included)."""
        if not _install_lock.acquire(blocking=False):
            return {"ok": False, "error": "an install/update is already running"}
        try:
            if not PAYLOAD_BIN.is_file():
                return {
                    "ok": False,
                    "error": "no embedded payload — the plugin was installed in dev mode; from a checkout run: just install-all",
                }
            status = await self.service_status()
            was_active = status.get("ActiveState") == "active"
            was_enabled = bool(status.get("enabled"))
            old_version = _binary_version()
            steps = []
            if was_active:
                _systemctl("stop", UNIT)
                steps.append("systemctl --user stop")
            bak = Path(f"{BINARY}.bak")
            if Path(BINARY).exists():
                shutil.copy2(BINARY, bak)
                steps.append(f"backup → {bak}")
            try:
                _copy_atomic(PAYLOAD_BIN, BINARY, 0o755)
                steps.append(f"binary → {BINARY}")
                _copy_atomic(PAYLOAD_UNIT, UNIT_FILE, 0o644)
                steps.append(f"unit → {UNIT_FILE}")
                if not CONFIG.exists():
                    _copy_atomic(PAYLOAD_CONFIG, CONFIG, 0o644)
                    steps.append(f"example config → {CONFIG}")
                if PAYLOAD_LICENSE.is_file():
                    _copy_atomic(PAYLOAD_LICENSE, LICENSE_DEST, 0o644)
                _install_desktop(steps)
                r = _systemctl("daemon-reload")
                if r.returncode != 0:
                    raise RuntimeError(f"daemon-reload failed: {r.stderr.strip()}")
                steps.append("systemctl --user daemon-reload")
                if was_enabled:
                    _systemctl("enable", UNIT)
                    steps.append("systemctl --user enable")
                if was_active:
                    r = _systemctl("start", UNIT)
                    if r.returncode != 0:
                        raise RuntimeError(f"start failed: {r.stderr.strip()}")
                    steps.append("systemctl --user start")
            except Exception as e:
                if bak.exists():
                    # Atomic rename (a plain copy could hit «text file busy»);
                    # the rollback consumes the .bak by design.
                    os.replace(bak, BINARY)
                    _systemctl("daemon-reload")
                    if was_active:
                        _systemctl("start", UNIT)
                    steps.append("rollback: previous binary restored from .bak")
                return {"ok": False, "error": str(e), "steps": steps}
            bak.unlink(missing_ok=True)
            return {"ok": True, "from": old_version, "to": _plugin_version(), "steps": steps}
        finally:
            _install_lock.release()

    async def capture_status(self):
        """Capture state: service ActiveState plus the capture phase parsed
        from the last journal lines (the daemon logs each transition:
        "found gamescope node" = capturing, "session ended"/"retrying"/
        "waiting for the gamescope node" = waiting, "capture paused"/
        "capture resumed" = paused/waiting, plus the live fps from the
        last stats line)."""
        status = await self.service_status()
        active = status.get("ActiveState") == "active"
        result = {"ok": True, "active": active, "state": "inactive", "fps": None, "detail": ""}
        if not active:
            result["detail"] = f"service {status.get('ActiveState', 'unknown')}"
            return result
        res = _journalctl("-n", "200")
        state = "waiting"
        fps = None
        detail = "no gamescope session seen yet"
        for line in (res.stdout + res.stderr).splitlines():
            if "capture paused" in line:
                state, detail = "paused", "paused (SIGUSR1)"
            elif "capture resumed" in line:
                state, detail = "waiting", "resumed; waiting for a gamescope session"
            elif "found gamescope node" in line:
                state = "capturing"
                m = re.search(r"id (\d+)", line)
                detail = f"capturing gamescope node id {m.group(1)}" if m else "capturing"
            elif "session ended" in line or "retrying in 2 s" in line or "waiting for the gamescope node" in line:
                state, detail = "waiting", "waiting for a gamescope session"
            m = re.search(r"([\d.]+) frames/s", line)
            if m:
                fps = float(m.group(1))
        result.update(state=state, fps=fps, detail=detail)
        return result

    async def capture_set_paused(self, paused: bool):
        """Toggle the daemon's manual capture pause with SIGUSR1.

        Refuses while the service is not active (the daemon owns the
        capture); only signals when the parsed current state differs
        from the requested one.
        """
        status = await self.service_status()
        if status.get("ActiveState") != "active":
            return {"ok": False, "error": "the service is not active; start it first"}
        current = await self.capture_status()
        if (current.get("state") == "paused") == paused:
            return {"ok": True, "state": current.get("state"), "already": True}
        res = _systemctl("kill", UNIT, "--signal=SIGUSR1")
        if res.returncode != 0:
            return {"ok": False, "error": res.stderr.strip() or "could not signal the daemon"}
        return {"ok": True}

    async def probe_wled(self, host):
        """Reachability check for the WLED controller: HTTP GET /json/info
        on port 80 (same probe as --setup). Drives the green OK / red NOK
        next to the host field on the configuration screen."""
        host = (host or "").strip()
        if not host:
            return {"ok": False, "error": "empty host"}
        info, err = _probe_wled(host)
        if err:
            return {"ok": False, "error": err}
        return {"ok": True, "info": info}

    async def wled_status(self):
        """Strip status for the main page: probe the configured host."""
        res = await self.get_config()
        host = ((res.get("config") or {}).get("wled") or {}).get("host")
        if not host:
            return {"ok": True, "configured": False, "reachable": False}
        probe = await self.probe_wled(host)
        return {
            "ok": True,
            "configured": True,
            "host": host,
            "reachable": probe["ok"],
            "info": probe.get("info"),
            "error": probe.get("error"),
        }

    async def preview_set(self, host, port, timeout_s, layout):
        """Start (or respawn) the --preview-strip process with the draft
        layout, so every change on the configuration screen has an
        immediate effect on the strip without writing the config file.
        The draft is validated first; an invalid draft is not spawned.
        The spawn command carries --top … --offset … --gap … as CLI flags."""
        global _preview_proc
        host = (host or "").strip()
        if not host:
            return {"ok": False, "errors": ["wled.host: empty host"]}
        merged = _deep_merge(
            DEFAULTS,
            {"wled": {"host": host, "port": port, "timeout_s": timeout_s}, "layout": layout},
        )
        errors = validate(merged)
        if errors:
            return {"ok": False, "errors": errors}
        if not os.path.exists(BINARY):
            return {"ok": False, "errors": [f"binary not found: {BINARY} (run install.sh first)"]}
        _stop_preview()
        args = [
            BINARY,
            "--preview-strip",
            "--wled", host,
            "--port", str(int(port)),
            "--timeout", str(int(timeout_s)),
            *_layout_args(layout),
        ]
        try:
            _preview_proc = subprocess.Popen(
                args,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                preexec_fn=_pdeathsig,
            )
        except OSError as e:
            _preview_proc = None
            return {"ok": False, "errors": [f"could not spawn {BINARY}: {e}"]}
        l = merged["layout"]
        # The offset is a circular shift and the gap a hole without LEDs:
        # the packet always carries the whole chain, so packet_len = count.
        return {"ok": True, "packet_len": sum(l[e] for e in ("top", "right", "bottom", "left"))}

    async def preview_stop(self):
        """Stop the strip preview (leaving the configuration screen)."""
        _stop_preview()
        return {"ok": True}

    async def get_logs(self, n: int = 200):
        res = _journalctl("-n", str(n))
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
