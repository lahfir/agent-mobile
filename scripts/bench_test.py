"""Stdlib tests for the scripts/benchkit benchmark harness."""
import contextlib
import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from scripts.benchkit import config, driver, flows, select  # noqa: E402


def dev(**kw):
    return kw


IOS = dev(name="iPhone 17", platform="ios", key="ios:UDID-1", id="UDID-1", udid="UDID-1", kind="simulator")
IOS2 = dev(name="iPhone 17 Pro", platform="ios", key="ios:UDID-2", id="UDID-2", udid="UDID-2", kind="simulator")
AND = dev(name="api37", platform="android", key="android:avd:api37", id="avd:api37", udid="avd:api37", kind="emulator")
LEGACY = dev(name="iPhone 17", udid="UDID-9", kind="simulator")  # old binary: no platform/key/id


class SelectDeviceTests(unittest.TestCase):
    def test_exact_key_wins(self):
        self.assertIs(select.select_device([IOS, AND], "android:avd:api37"), AND)

    def test_stable_id_match(self):
        self.assertIs(select.select_device([IOS, IOS2], "UDID-2"), IOS2)

    def test_legacy_udid_match(self):
        self.assertIs(select.select_device([LEGACY], "UDID-9"), LEGACY)

    def test_name_match(self):
        self.assertIs(select.select_device([IOS, IOS2], "iPhone 17 Pro"), IOS2)

    def test_ambiguous_returns_none(self):
        dup_a = dev(name="iPhone 17", key="ios:A", id="A", udid="A")
        dup_b = dev(name="iPhone 17", key="ios:B", id="B", udid="B")
        self.assertIsNone(select.select_device([dup_a, dup_b], "iPhone 17"))

    def test_no_match_returns_none(self):
        self.assertIsNone(select.select_device([IOS], "nope"))


class BenchTargetTests(unittest.TestCase):
    def test_ios_row_yields_udid_boot(self):
        self.assertEqual(select.bench_target(IOS, "iPhone 17"), ("ios", "UDID-1", "UDID-1"))

    def test_legacy_row_without_platform_stays_ios(self):
        self.assertEqual(select.bench_target(LEGACY, "UDID-9"), ("ios", "UDID-9", "UDID-9"))

    def test_android_row_accepted_with_avd_boot(self):
        # The old iOS-only contract rejected this row; the agnostic harness
        # benches it, keyed by canonical key and booted by AVD name.
        self.assertEqual(
            select.bench_target(AND, "android:avd:api37"), ("android", "android:avd:api37", "api37")
        )

    def test_android_serial_row_defers_boot_target(self):
        usb = dev(name="usb", platform="android", key="android:ABC123", id="ABC123", kind="usb")
        self.assertEqual(select.bench_target(usb, "ABC123"), ("android", "android:ABC123", None))

    def test_unknown_platform_rejected(self):
        with self.assertRaises(SystemExit):
            select.bench_target(dev(name="x", platform="fuchsia", key="fuchsia:x"), "x")

    def test_missing_row_rejected(self):
        with self.assertRaises(SystemExit):
            select.bench_target(None, "nope")

    def test_ios_row_without_id_rejected(self):
        with self.assertRaises(SystemExit):
            select.bench_target(dev(name="x", platform="ios", key="ios:x"), "x")


class StopDriverTests(unittest.TestCase):
    def test_waits_for_recorded_pid_to_exit(self):
        # Reparented sleeper (no zombie: grandchildren reparent to init).
        # Detached fds: otherwise the sleeper inherits the capture pipes and
        # run() blocks until it exits naturally — a vacuous 60 s pass.
        out = subprocess.run(["bash", "-c", "sleep 60 </dev/null &>/dev/null & echo $!"],
                             capture_output=True, text=True)
        pid = int(out.stdout.strip())
        try:
            with tempfile.TemporaryDirectory() as tmp:
                state = pathlib.Path(tmp) / "state.json"
                state.write_text(json.dumps({"devices": {"k": {"pid": pid}}}))
                driver.stop_driver("k", state_path=state)
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
        finally:
            with contextlib.suppress(ProcessLookupError):
                os.kill(pid, signal.SIGKILL)

    def test_sigkill_fallback_for_term_immune_pid(self):
        out = subprocess.run(["bash", "-c", "trap '' TERM; sleep 60 </dev/null &>/dev/null & echo $!"],
                             capture_output=True, text=True)
        pid = int(out.stdout.strip())
        try:
            with tempfile.TemporaryDirectory() as tmp:
                state = pathlib.Path(tmp) / "state.json"
                state.write_text(json.dumps({"devices": {"k": {"pid": pid}}}))
                driver.stop_driver("k", state_path=state, term_grace=2)
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
        finally:
            with contextlib.suppress(ProcessLookupError):
                os.kill(pid, signal.SIGKILL)

    def test_missing_pid_is_a_noop(self):
        with tempfile.TemporaryDirectory() as tmp:
            state = pathlib.Path(tmp) / "state.json"
            state.write_text(json.dumps({"devices": {}}))
            driver.stop_driver("k", state_path=state)

    def test_missing_state_file_is_a_noop(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver.stop_driver("k", state_path=pathlib.Path(tmp) / "absent.json")


class ConfShapeTests(unittest.TestCase):
    def test_every_platform_conf_has_every_key(self):
        for platform, conf in config.CONF.items():
            for key in config.CONF_KEYS:
                self.assertIn(key, conf, f"{platform} conf lacks {key}")

    def test_every_platform_has_five_apps(self):
        for platform, apps in config.APPS.items():
            self.assertEqual(len(apps), 5, platform)


class GestureTests(unittest.TestCase):
    class FakeBench:
        conf = {"resnap": True, "settings": "pkg"}

        def __init__(self, screens):
            self.screens, self.calls = screens, []

        def run(self, *args, tag=None, i=None):
            self.calls.append(tag or args[0])
            return {"data": {"text": self.screens.pop(0)}}

        def act(self, env, verb, role, name, *extra, tag=None, i=None):
            self.calls.append(verb if f'{role} "{name}"' in env["data"]["text"] else "NO_TARGET")

    def test_relaunches_before_acting_when_the_target_left(self):
        b = self.FakeBench(['@s:e1 textfield "q"', '@s:e2 text "Search"'])
        flows.gesture(b, {}, "hold", "text", "Search")
        self.assertEqual(b.calls, ["resnap", "relaunch", "hold"])

    def test_acts_on_the_resnap_when_the_target_is_there(self):
        b = self.FakeBench(['@s:e2 text "Search"'])
        flows.gesture(b, {}, "hold", "text", "Search")
        self.assertEqual(b.calls, ["resnap", "hold"])


class AndroidAvdNameTests(unittest.TestCase):
    def test_avd_prefixed_id(self):
        self.assertEqual(select.android_avd_name(AND), "api37")

    def test_avd_prefixed_udid_fallback(self):
        row = dev(name="e", platform="android", id="emulator-5554", udid="avd:api37")
        self.assertEqual(select.android_avd_name(row), "api37")

    def test_bare_serial_yields_none(self):
        row = dev(name="e", platform="android", id="emulator-5554", kind="emulator")
        self.assertIsNone(select.android_avd_name(row))

    def test_missing_ids_yield_none(self):
        self.assertIsNone(select.android_avd_name(dev(name="x")))




class CanonicalSelectionTest(unittest.TestCase):
    def test_display_name_becomes_canonical_key(self):
        row = {"name": "iPhone 17", "key": "ios:UDID-1", "id": "UDID-1", "platform": "ios"}
        self.assertEqual(("ios:UDID-1", True), select.canonical_bench_selection(row, "iPhone 17"))

    def test_key_stays_key(self):
        row = {"name": "iPhone 17", "key": "ios:UDID-1", "id": "UDID-1", "platform": "ios"}
        self.assertEqual(("ios:UDID-1", True), select.canonical_bench_selection(row, "ios:UDID-1"))

    def test_legacy_row_keeps_selector_pinned(self):
        row = {"name": "iPhone 17", "udid": "UDID-1"}
        self.assertEqual(("iPhone 17", True), select.canonical_bench_selection(row, "iPhone 17"))

    def test_none_selector_with_key_resolves_key(self):
        row = {"name": "iPhone 17", "key": "ios:UDID-1"}
        self.assertEqual(("ios:UDID-1", True), select.canonical_bench_selection(row, None))

    def test_none_selector_without_key_stays_none(self):
        self.assertEqual((None, False), select.canonical_bench_selection({"name": "x"}, None))


if __name__ == "__main__":
    unittest.main()
