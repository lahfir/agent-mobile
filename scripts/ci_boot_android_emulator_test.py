#!/usr/bin/env python3
"""Hermetic coverage for ci-boot-android-emulator.sh ownership probes.

Fakes `adb`, `emulator`, and `timeout` under a temp PATH/SDK so the real
script's fail-closed identity path is exercised without touching any
runtime.
"""
import os
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "ci-boot-android-emulator.sh"

ADB_SH = """#!/bin/sh
case "$*" in
  *"devices"*) %(devices)s ;;
  *"emu avd name"*) %(avd_name)s ;;
esac
exit 0
"""

EMU_SH = """#!/bin/sh
case "$1" in
  -accel-check) exit 0 ;;
  -list-avds) echo "agent-mobile-api37" ;;
  -avd) touch "$AM_FAKE_SPAWN_MARKER" ;;
esac
exit 0
"""

TIMEOUT_SH = "#!/bin/sh\nshift\nexec \"$@\"\n"

UNAME_SH = "#!/bin/sh\necho Darwin\n"


def write_exe(path, text):
    path.write_text(text)
    path.chmod(0o700)


def make_env(tmp, avd_name_behavior="exit 3", devices_behavior=None):
    sdk = tmp / "sdk"
    (sdk / "platform-tools").mkdir(parents=True)
    (sdk / "emulator").mkdir(parents=True)
    fakes = tmp / "bin"
    fakes.mkdir(exist_ok=True)
    write_exe(fakes / "timeout", TIMEOUT_SH)
    write_exe(fakes / "uname", UNAME_SH)
    write_exe(
        sdk / "platform-tools" / "adb",
        ADB_SH
        % {
            "devices": devices_behavior
            or "printf 'List of devices attached\\nemulator-5554\\tdevice\\n'",
            "avd_name": avd_name_behavior,
        },
    )
    write_exe(sdk / "emulator" / "emulator", EMU_SH)
    for sub in ("home", "avd", "rt"):
        (tmp / sub).mkdir(exist_ok=True)
    (tmp / "avd" / "agent-mobile-api37.avd").mkdir(exist_ok=True)
    return dict(
        os.environ,
        PATH=f"{fakes}:{os.environ['PATH']}",
        ANDROID_HOME=str(sdk),
        ANDROID_SDK_ROOT=str(sdk),
        ANDROID_AVD_HOME=str(tmp / "avd"),
        RUNNER_TEMP=str(tmp / "rt"),
        HOME=str(tmp / "home"),
        AM_FAKE_SPAWN_MARKER=str(tmp / "spawned"),
    )


def run_script(args, env):
    return subprocess.run(
        ["bash", str(SCRIPT)] + args,
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )


class CiBootTests(unittest.TestCase):
    def test_unprovable_identity_aborts_before_spawn(self):
        with tempfile.TemporaryDirectory(prefix="am-ci-boot-") as td:
            tmp = pathlib.Path(td)
            env = make_env(tmp, "exit 3")
            proc = run_script(["start"], env)
            self.assertNotEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("cannot prove AVD identity", proc.stderr)
            self.assertFalse((tmp / "spawned").exists(), "emulator must never spawn unproven")

    def test_running_same_name_emulator_is_rejected_before_spawn(self):
        with tempfile.TemporaryDirectory(prefix="am-ci-boot-") as td:
            tmp = pathlib.Path(td)
            env = make_env(tmp, "echo agent-mobile-api37")
            proc = run_script(["start"], env)
            self.assertNotEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("already runs", proc.stderr)
            self.assertFalse((tmp / "spawned").exists())

    def test_stop_fails_closed_when_adb_missing(self):
        with tempfile.TemporaryDirectory(prefix="am-ci-boot-") as td:
            tmp = pathlib.Path(td)
            env = make_env(tmp, "exit 3")
            os.unlink(pathlib.Path(env["ANDROID_HOME"]) / "platform-tools" / "adb")
            state_dir = pathlib.Path(env["RUNNER_TEMP"]) / "agent-mobile-android-ci"
            state_dir.mkdir(parents=True)
            (state_dir / "serial").write_text("emulator-5554\n")
            proc = run_script(["stop"], env)
            self.assertNotEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("adb unavailable for the recorded serial", proc.stderr)
            self.assertTrue(state_dir.exists(), "failed stop must retain state")

    def test_stop_fails_closed_when_devices_probe_fails(self):
        with tempfile.TemporaryDirectory(prefix="am-ci-boot-") as td:
            tmp = pathlib.Path(td)
            env = make_env(tmp, "exit 3", devices_behavior="exit 2")
            state_dir = pathlib.Path(env["RUNNER_TEMP"]) / "agent-mobile-android-ci"
            state_dir.mkdir(parents=True)
            (state_dir / "serial").write_text("emulator-5554\n")
            proc = run_script(["stop"], env)
            self.assertNotEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("adb devices failed", proc.stderr)
            self.assertTrue(state_dir.exists(), "failed stop must retain state")

    def test_stop_refuses_symlinked_state_dir(self):
        with tempfile.TemporaryDirectory(prefix="am-ci-boot-") as td:
            tmp = pathlib.Path(td)
            env = make_env(tmp, "exit 3", devices_behavior="exit 0")
            state_dir = pathlib.Path(env["RUNNER_TEMP"]) / "agent-mobile-android-ci"
            foreign = tmp / "foreign"
            foreign.mkdir()
            (foreign / "serial").write_text("emulator-5554\n")
            os.symlink(foreign, state_dir)
            proc = run_script(["stop"], env)
            self.assertNotEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("ownership", proc.stderr)
            self.assertTrue(state_dir.is_symlink(), "symlink must remain")
            self.assertTrue((foreign / "serial").exists(), "foreign dir untouched")

    def test_stop_delete_avd_outside_runner_temp_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="am-ci-boot-") as td:
            tmp = pathlib.Path(td)
            env = make_env(tmp, "exit 3", devices_behavior="exit 0")
            env["CI_ANDROID_DELETE_AVD"] = "1"
            state_dir = pathlib.Path(env["RUNNER_TEMP"]) / "agent-mobile-android-ci"
            state_dir.mkdir(parents=True)
            (state_dir / "serial").write_text("emulator-5554\n")
            avd_dir = pathlib.Path(env["ANDROID_AVD_HOME"]) / "agent-mobile-api37.avd"
            proc = run_script(["stop"], env)
            self.assertNotEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("runner temp", proc.stderr)
            self.assertTrue(state_dir.exists(), "failed delete must retain state")
            self.assertTrue(avd_dir.exists(), "AVD dir must remain")


if __name__ == "__main__":
    unittest.main()
