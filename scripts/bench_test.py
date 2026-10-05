"""Stdlib tests for scripts/bench.py device selection."""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import bench  # noqa: E402


def dev(**kw):
    return kw


IOS = dev(name="iPhone 17", platform="ios", key="ios:UDID-1", id="UDID-1", udid="UDID-1", kind="simulator")
IOS2 = dev(name="iPhone 17 Pro", platform="ios", key="ios:UDID-2", id="UDID-2", udid="UDID-2", kind="simulator")
AND = dev(name="api37", platform="android", key="android:avd:api37", id="avd:api37", udid="avd:api37", kind="emulator")
LEGACY = dev(name="iPhone 17", udid="UDID-9", kind="simulator")  # old binary: no platform/key/id


class SelectDeviceTests(unittest.TestCase):
    def test_exact_key_wins(self):
        self.assertIs(bench.select_device([IOS, AND], "android:avd:api37"), AND)

    def test_stable_id_match(self):
        self.assertIs(bench.select_device([IOS, IOS2], "UDID-2"), IOS2)

    def test_legacy_udid_match(self):
        self.assertIs(bench.select_device([LEGACY], "UDID-9"), LEGACY)

    def test_name_match(self):
        self.assertIs(bench.select_device([IOS, IOS2], "iPhone 17 Pro"), IOS2)

    def test_ambiguous_returns_none(self):
        dup_a = dev(name="iPhone 17", key="ios:A", id="A", udid="A")
        dup_b = dev(name="iPhone 17", key="ios:B", id="B", udid="B")
        self.assertIsNone(bench.select_device([dup_a, dup_b], "iPhone 17"))

    def test_no_match_returns_none(self):
        self.assertIsNone(bench.select_device([IOS], "nope"))


class SimulatorUdidTests(unittest.TestCase):
    def test_ios_row_uses_id(self):
        self.assertEqual(bench.simulator_udid(IOS, "iPhone 17"), "UDID-1")

    def test_legacy_row_without_platform_still_yields_udid(self):
        self.assertEqual(bench.simulator_udid(LEGACY, "UDID-9"), "UDID-9")

    def test_android_platform_rejected(self):
        with self.assertRaises(SystemExit):
            bench.simulator_udid(AND, "android:avd:api37")

    def test_missing_udid_rejected(self):
        with self.assertRaises(SystemExit):
            bench.simulator_udid(dev(name="x", platform="ios", key="ios:x"), "x")




class CanonicalSelectionTest(unittest.TestCase):
    def test_display_name_becomes_canonical_key(self):
        row = {"name": "iPhone 17", "key": "ios:UDID-1", "id": "UDID-1", "platform": "ios"}
        self.assertEqual(("ios:UDID-1", True), bench.canonical_bench_selection(row, "iPhone 17"))

    def test_key_stays_key(self):
        row = {"name": "iPhone 17", "key": "ios:UDID-1", "id": "UDID-1", "platform": "ios"}
        self.assertEqual(("ios:UDID-1", True), bench.canonical_bench_selection(row, "ios:UDID-1"))

    def test_legacy_row_keeps_selector_pinned(self):
        row = {"name": "iPhone 17", "udid": "UDID-1"}
        self.assertEqual(("iPhone 17", True), bench.canonical_bench_selection(row, "iPhone 17"))

    def test_none_selector_with_key_resolves_key(self):
        row = {"name": "iPhone 17", "key": "ios:UDID-1"}
        self.assertEqual(("ios:UDID-1", True), bench.canonical_bench_selection(row, None))

    def test_none_selector_without_key_stays_none(self):
        self.assertEqual((None, False), bench.canonical_bench_selection({"name": "x"}, None))


if __name__ == "__main__":
    unittest.main()
