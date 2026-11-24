"""Unit tests for the install/update logic of plugin/main.py (SPEC §11).

The module-level path constants of main are redirected into a temp dir, the
systemctl wrapper is replaced by a fake, and binaries are tiny shell scripts
answering --version — no systemd, no root, no real binary needed.
"""

import asyncio
import json
import os
import sys
import tempfile
import threading
import unittest
from pathlib import Path

PLUGIN_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PLUGIN_ROOT))

import main  # noqa: E402


def make_fake_binary(path, version):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f'#!/bin/sh\necho "gamescope-led-sync {version}"\n', encoding="utf-8")
    path.chmod(0o755)


class FakeResult:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


class FakeSystemctl:
    """Records calls; answers `show` / `is-enabled` from its state, and fails
    any call whose verb is listed in fail_on."""

    def __init__(self, active=False, enabled=False, fail_on=()):
        self.calls = []
        self.active = active
        self.enabled = enabled
        self.fail_on = tuple(fail_on)

    def __call__(self, *args):
        self.calls.append(args)
        if args[0] == "show":
            state = "active" if self.active else "inactive"
            sub = "running" if self.active else "dead"
            unit = "enabled" if self.enabled else "disabled"
            return FakeResult(0, f"ActiveState={state}\nSubState={sub}\nUnitFileState={unit}\n")
        if args[0] == "is-enabled":
            return FakeResult(0 if self.enabled else 1)
        if any(f in args for f in self.fail_on):
            return FakeResult(1, "", "boom")
        return FakeResult()

    def verbs(self):
        return [c[0] for c in self.calls]


PATH_CONSTANTS = (
    "PLUGIN_DIR",
    "PAYLOAD",
    "PAYLOAD_BIN",
    "PAYLOAD_UNIT",
    "PAYLOAD_CONFIG",
    "PAYLOAD_LICENSE",
    "CONFIG",
    "BINARY",
    "BIN_DIR",
    "UNIT_DIR",
    "UNIT_FILE",
    "LICENSE_DEST",
)


class InstallTestCase(unittest.TestCase):
    """Redirects main's paths into a temp dir and fakes systemctl."""

    PLUGIN_VERSION = "0.2.0"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self._saved = {n: getattr(main, n) for n in (*PATH_CONSTANTS, "_systemctl", "_install_lock")}
        main.PLUGIN_DIR = root / "plugin"
        main.PAYLOAD = main.PLUGIN_DIR / "payload"
        main.PAYLOAD_BIN = main.PAYLOAD / "bin" / "gamescope-led-sync"
        main.PAYLOAD_UNIT = main.PAYLOAD / "systemd" / "gamescope-led-sync.service"
        main.PAYLOAD_CONFIG = main.PAYLOAD / "config" / "config.toml.example"
        main.PAYLOAD_LICENSE = main.PAYLOAD / "LICENSE"
        main.CONFIG = root / ".config" / "gamescope-led-sync" / "config.toml"
        main.BINARY = str(root / ".local" / "bin" / "gamescope-led-sync")
        main.BIN_DIR = root / ".local" / "bin"
        main.UNIT_DIR = root / ".config" / "systemd" / "user"
        main.UNIT_FILE = main.UNIT_DIR / "gamescope-led-sync.service"
        main.LICENSE_DEST = root / ".local" / "share" / "gamescope-led-sync" / "LICENSE"
        main._install_lock = threading.Lock()
        main.PLUGIN_DIR.mkdir(parents=True, exist_ok=True)
        (main.PLUGIN_DIR / "plugin.json").write_text(
            json.dumps({"name": "Gamescope LED Sync", "version": self.PLUGIN_VERSION}),
            encoding="utf-8",
        )
        self.systemctl = FakeSystemctl()
        main._systemctl = self.systemctl
        self.plugin = main.Plugin()

    def tearDown(self):
        for name, value in self._saved.items():
            setattr(main, name, value)
        self.tmp.cleanup()

    def go(self, coro):
        return asyncio.run(coro)

    def make_payload(self, version=None):
        version = version or self.PLUGIN_VERSION
        make_fake_binary(main.PAYLOAD_BIN, version)
        main.PAYLOAD_UNIT.parent.mkdir(parents=True, exist_ok=True)
        main.PAYLOAD_UNIT.write_text("[Unit]\nDescription=payload unit\n", encoding="utf-8")
        main.PAYLOAD_CONFIG.parent.mkdir(parents=True, exist_ok=True)
        main.PAYLOAD_CONFIG.write_text("[wled]\nhost = \"wled.local\"\n", encoding="utf-8")
        main.PAYLOAD_LICENSE.write_text("Apache-2.0\n", encoding="utf-8")

    def install_binary(self, version):
        make_fake_binary(main.BINARY, version)


class VersionTests(InstallTestCase):
    def test_binary_version_parses_the_version_line(self):
        self.install_binary("0.1.0")
        self.assertEqual(main._binary_version(), "0.1.0")

    def test_binary_version_none_when_missing(self):
        self.assertIsNone(main._binary_version())

    def test_binary_version_none_when_not_executable(self):
        path = Path(main.BINARY)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\necho 'gamescope-led-sync 0.1.0'\n", encoding="utf-8")
        path.chmod(0o644)
        self.assertIsNone(main._binary_version())

    def test_plugin_version_comes_from_plugin_json(self):
        self.assertEqual(main._plugin_version(), self.PLUGIN_VERSION)


class InstallStatusTests(InstallTestCase):
    def test_nothing_installed(self):
        st = self.go(self.plugin.install_status())
        self.assertTrue(st["ok"])
        self.assertEqual(st["plugin_version"], self.PLUGIN_VERSION)
        self.assertFalse(st["payload_present"])
        self.assertFalse(st["binary_installed"])
        self.assertFalse(st["unit_installed"])
        self.assertIsNone(st["versions_match"])
        self.assertFalse(st["update_needed"])

    def test_mismatch_means_update_needed(self):
        self.install_binary("0.1.0")
        st = self.go(self.plugin.install_status())
        self.assertTrue(st["binary_installed"])
        self.assertEqual(st["binary_version"], "0.1.0")
        self.assertFalse(st["versions_match"])
        self.assertTrue(st["update_needed"])

    def test_match_means_no_update(self):
        self.install_binary(self.PLUGIN_VERSION)
        st = self.go(self.plugin.install_status())
        self.assertTrue(st["versions_match"])
        self.assertFalse(st["update_needed"])

    def test_newer_binary_is_a_mismatch_too(self):
        # A downgrade (old zip re-imported) must also offer «Mettre à jour».
        self.install_binary("9.9.9")
        st = self.go(self.plugin.install_status())
        self.assertTrue(st["update_needed"])

    def test_broken_binary_is_a_mismatch(self):
        path = Path(main.BINARY)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("not a binary", encoding="utf-8")
        path.chmod(0o755)
        st = self.go(self.plugin.install_status())
        self.assertTrue(st["binary_installed"])
        self.assertIsNone(st["binary_version"])
        self.assertTrue(st["update_needed"])


class ServiceInstallTests(InstallTestCase):
    def test_install_copies_payload_enables_and_starts(self):
        self.make_payload()
        res = self.go(self.plugin.service_install())
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["version"], self.PLUGIN_VERSION)
        self.assertTrue(os.access(main.BINARY, os.X_OK))
        self.assertEqual(main._binary_version(), self.PLUGIN_VERSION)
        self.assertTrue(main.UNIT_FILE.is_file())
        self.assertTrue(main.CONFIG.is_file())
        self.assertTrue(main.LICENSE_DEST.is_file())
        verbs = self.systemctl.verbs()
        self.assertIn("daemon-reload", verbs)
        self.assertIn("enable", verbs)  # enable --now: boot + start
        # atomic copy: no temp file left behind
        self.assertEqual(list(main.BIN_DIR.glob(".*.tmp")), [])

    def test_install_never_overwrites_an_existing_config(self):
        self.make_payload()
        main.CONFIG.parent.mkdir(parents=True, exist_ok=True)
        main.CONFIG.write_text("# my config\n", encoding="utf-8")
        res = self.go(self.plugin.service_install())
        self.assertTrue(res["ok"], res)
        self.assertEqual(main.CONFIG.read_text(encoding="utf-8"), "# my config\n")

    def test_install_without_payload_fails_with_dev_hint(self):
        res = self.go(self.plugin.service_install())
        self.assertFalse(res["ok"])
        self.assertIn("payload", res["error"])
        self.assertIn("install-all", res["error"])


class ServiceUpdateTests(InstallTestCase):
    def test_update_replaces_binary_and_restarts_as_before(self):
        self.install_binary("0.1.0")
        main.UNIT_DIR.mkdir(parents=True, exist_ok=True)
        main.UNIT_FILE.write_text("[Unit]\nDescription=old\n", encoding="utf-8")
        self.systemctl.active = True
        self.systemctl.enabled = True
        self.make_payload()
        res = self.go(self.plugin.service_update())
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["from"], "0.1.0")
        self.assertEqual(res["to"], self.PLUGIN_VERSION)
        self.assertEqual(main._binary_version(), self.PLUGIN_VERSION)
        self.assertEqual(main.UNIT_FILE.read_text(encoding="utf-8"), "[Unit]\nDescription=payload unit\n")
        verbs = self.systemctl.verbs()
        self.assertEqual(verbs.count("stop"), 1)
        self.assertIn("daemon-reload", verbs)
        self.assertIn("enable", verbs)
        self.assertIn("start", verbs)
        # .bak cleaned up on success
        self.assertFalse(Path(main.BINARY + ".bak").exists())

    def test_update_rolls_back_when_start_fails(self):
        self.install_binary("0.1.0")
        old_bytes = Path(main.BINARY).read_bytes()
        self.systemctl.active = True
        self.systemctl.fail_on = ("start",)
        self.make_payload()
        res = self.go(self.plugin.service_update())
        self.assertFalse(res["ok"])
        self.assertIn("start failed", res["error"])
        # the old binary is restored (the rollback consumes the .bak)
        self.assertEqual(Path(main.BINARY).read_bytes(), old_bytes)
        self.assertFalse(Path(main.BINARY + ".bak").exists())
        self.assertTrue(any("rollback" in s for s in res["steps"]))

    def test_update_without_payload_fails(self):
        self.install_binary("0.1.0")
        res = self.go(self.plugin.service_update())
        self.assertFalse(res["ok"])
        self.assertIn("payload", res["error"])


class ServiceUninstallTests(InstallTestCase):
    def test_uninstall_removes_unit_and_binary_but_keeps_config(self):
        self.install_binary(self.PLUGIN_VERSION)
        main.UNIT_DIR.mkdir(parents=True, exist_ok=True)
        main.UNIT_FILE.write_text("[Unit]\n", encoding="utf-8")
        main.CONFIG.parent.mkdir(parents=True, exist_ok=True)
        main.CONFIG.write_text("# keep me\n", encoding="utf-8")
        res = self.go(self.plugin.service_uninstall())
        self.assertTrue(res["ok"], res)
        self.assertEqual(res["config_kept"], str(main.CONFIG))
        self.assertFalse(Path(main.BINARY).exists())
        self.assertFalse(main.UNIT_FILE.exists())
        self.assertEqual(main.CONFIG.read_text(encoding="utf-8"), "# keep me\n")
        verbs = self.systemctl.verbs()
        self.assertIn("disable", verbs)
        self.assertIn("daemon-reload", verbs)


if __name__ == "__main__":
    unittest.main()
