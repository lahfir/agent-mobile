"""Hermetic tests for setup-android-sdk.sh revision pinning and
run_with_deadline.py group cleanup. Everything is temp-dir/fake-binary —
no real SDK, adb, or emulator is touched.
"""

import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SETUP = ROOT / "scripts" / "setup-android-sdk.sh"
DEADLINE = ROOT / "scripts" / "run_with_deadline.py"

CMDTOOLS_BUILD = "15859902"


def fake_sdkmanager(home: Path, installed: dict, available: dict) -> None:
    """Fake sdkmanager: seeded tables, logs calls, and 'installs' by
    appending rows to its own state file so post-install verification
    sees real mutation."""
    bin_dir = home / "cmdline-tools" / CMDTOOLS_BUILD / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    state = home / "installed.txt"
    state.write_text("".join(f"{i}|{r}\n" for i, r in installed.items()))
    available_rows = "".join(f"{i} | {r}\n" for i, r in available.items())
    avail_map = home / "available.txt"
    avail_map.write_text("".join(f"{i}|{r}\n" for i, r in available.items()))
    script = bin_dir / "sdkmanager"
    script.write_text(
        "#!/usr/bin/env bash\n"
        + f"echo \"$*\" >> '{home}/invocations.log'\n"
        + 'case "$*" in\n'
        + f"    *--list_installed*) awk -F'|' '{{print $1 \042 | \042 $2}}' '{state}' ;;\n"
        + f"    *--list*) printf '%s' '{available_rows}' ;;\n"
        + f"    *--licenses*) ;;\n"
        + f"    *) for p in \"$@\"; do awk -F'|' -v p=\"$p\" '$1==p{{print p \042|\042 $2}}' '{avail_map}' >> '{state}'; done ;;\n"
        + "esac\nexit 0\n"
    )
    script.chmod(0o755)


def run_setup(home: Path, body: str, env_overrides: dict | None = None) -> subprocess.CompletedProcess:
    env = dict(os.environ, ANDROID_HOME=str(home))
    if env_overrides:
        for key, value in env_overrides.items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = value
    cmd = f"set -e; source '{SETUP}'; detect_host; {body}"
    return subprocess.run(
        ["bash", "-c", cmd], env=env, capture_output=True, text=True, timeout=30
    )


def all_pinned_rows() -> dict:
    """Every required singleton at its pin, plus BOTH sys-image ABIs so
    the fixture is host-arch independent."""
    return {
        "platform-tools": "37.0.1",
        "platforms;android-37.0": "2",
        "build-tools;36.0.0": "36.0.0",
        "emulator": "37.2.12",
        "system-images;android-37.0;google_apis;arm64-v8a": "6",
        "system-images;android-37.0;google_apis;x86_64": "6",
    }


class CmdlineToolsRevisionTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def test_reads_exact_installed_and_staging_dirs(self):
        installed = self.home / "cmdline-tools" / CMDTOOLS_BUILD
        staging = self.home / ".staging" / "cmdline-tools"
        for d in (installed, staging):
            d.mkdir(parents=True)
            (d / "source.properties").write_text("Pkg.Revision=22.0\n")
        body = (
            "cmdline_tools_revision "
            f"'{installed}'; cmdline_tools_revision '{staging}'"
        )
        out = run_setup(self.home, body)
        self.assertEqual(0, out.returncode, out.stderr)
        self.assertEqual("22.0\n22.0", out.stdout.strip())


class InstallPackagesTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def invocations(self) -> str:
        log = self.home / "invocations.log"
        return log.read_text() if log.exists() else ""

    def test_exact_revisions_do_nothing(self):
        fake_sdkmanager(self.home, all_pinned_rows(), {})
        out = run_setup(self.home, "install_packages")
        self.assertEqual(0, out.returncode, out.stderr)
        calls = self.invocations().splitlines()
        self.assertTrue(calls, "sdkmanager must at least be queried")
        self.assertTrue(
            all("--list_installed" in c for c in calls),
            f"only --list_installed may run: {calls}",
        )

    def test_installed_revision_mismatch_fails_before_mutation(self):
        installed = all_pinned_rows()
        installed["platform-tools"] = "36.0.0"
        fake_sdkmanager(self.home, installed, {})
        out = run_setup(self.home, "install_packages")
        self.assertNotEqual(0, out.returncode)
        self.assertIn("platform-tools", out.stderr)
        self.assertNotIn("--licenses", self.invocations())

    def test_missing_package_advertised_mismatch_fails(self):
        installed = all_pinned_rows()
        installed.pop("platform-tools")
        available = {"platform-tools": "99.0.0"}
        fake_sdkmanager(self.home, installed, available)
        out = run_setup(self.home, "install_packages")
        self.assertNotEqual(0, out.returncode)
        self.assertNotIn("--licenses", self.invocations())

    def test_missing_package_with_exact_advertised_installs(self):
        installed = all_pinned_rows()
        installed.pop("platform-tools")
        available = {"platform-tools": "37.0.1"}
        fake_sdkmanager(self.home, installed, available)
        out = run_setup(self.home, "install_packages")
        self.assertEqual(0, out.returncode, out.stderr)
        self.assertIn("--licenses", self.invocations())
        install_lines = [
            l for l in self.invocations().splitlines()
            if "--list" not in l and "--licenses" not in l
        ]
        self.assertEqual(1, len(install_lines))
        self.assertIn("platform-tools", install_lines[0])


class DeadlineWrapperTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.parent_pid = self.dir / "parent.pid"
        self.desc_pid = self.dir / "descendant.pid"
        self.ready = self.dir / "descendant.ready"
        child = self.dir / "descendant.py"
        child.write_text(textwrap.dedent(f"""\
            import os, signal, time
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGHUP, signal.SIG_IGN)
            open('{self.desc_pid}', 'w').write(str(os.getpid()))
            open('{self.ready}', 'w').write('1')
            time.sleep(60)
            """))
        parent = self.dir / "parent.py"
        parent.write_text(textwrap.dedent(f"""\
            import os, subprocess, sys, time
            child = subprocess.Popen([sys.executable, '{child}'])
            deadline = time.time() + 10
            while not os.path.exists('{self.ready}') and time.time() < deadline:
                time.sleep(0.02)
            open('{self.parent_pid}', 'w').write(str(os.getpid()))
            time.sleep(60)
            """))
        self.helper = parent

    def tearDown(self):
        self._tmp.cleanup()

    def spawn(self, secs: str) -> subprocess.Popen:
        proc = subprocess.Popen(
            [sys.executable, str(DEADLINE), secs, sys.executable, str(self.helper)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        limit = time.time() + 10
        while not (self.parent_pid.exists() and self.desc_pid.exists()) \
                and time.time() < limit:
            time.sleep(0.05)
        assert self.parent_pid.exists() and self.desc_pid.exists()
        return proc

    @staticmethod
    def dead_or_zombie(pid: int) -> bool:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        except PermissionError:
            pass
        out = subprocess.run(
            ["ps", "-p", str(pid), "-o", "state="],
            capture_output=True, text=True, timeout=5,
        ).stdout
        return "Z" in out or out.strip() == ""

    def assert_group_gone(self):
        parent = int(self.parent_pid.read_text())
        desc = int(self.desc_pid.read_text())
        limit = time.time() + 15
        while (not self.dead_or_zombie(parent) or not self.dead_or_zombie(desc)) \
                and time.time() < limit:
            time.sleep(0.1)
        self.assertTrue(self.dead_or_zombie(parent), f"parent {parent} survived")
        self.assertTrue(self.dead_or_zombie(desc), f"descendant {desc} survived")

    def test_sigterm_reaps_ignoring_descendant(self):
        proc = self.spawn("60")
        proc.send_signal(signal.SIGTERM)
        self.assertEqual(143, proc.wait(timeout=20))
        self.assert_group_gone()

    def test_timeout_reaps_ignoring_descendant(self):
        proc = self.spawn("0.5")
        self.assertEqual(124, proc.wait(timeout=20))
        self.assert_group_gone()


def host_abi() -> str:
    """ABI this test host's detect_host would pin."""
    if sys.platform == "darwin" and os.uname().machine == "arm64":
        return "arm64-v8a"
    return "x86_64"


def fake_avdmanager(sdk: Path, log: Path, *, write_config: bool = True,
                    write_metadata: bool = True) -> None:
    """Fake avdmanager modelling pinned CLI metadata/data separation:
    metadata root is ANDROID_AVD_HOME when it already exists as a
    directory, else HOME/.android/avd; metadata is `<root>/<name>.ini`
    pointing at the data folder; --path selects the data folder itself.
    write_config/write_metadata toggle each side independently."""
    bin_dir = sdk / "cmdline-tools" / CMDTOOLS_BUILD / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    script = bin_dir / "avdmanager"
    meta_cfg = (
        "printf 'path=%s\\ntarget=android-37.0\\n' \"$avddir\" > \"$base/$name.ini\"\n"
        if write_metadata else ""
    )
    data_cfg = (
        "printf 'abi.type=" + host_abi() + "\\n"
        "hw.device.name=pixel_7\\n"
        "image.sysdir.1=system-images/android-37.0/google_apis/" + host_abi() + "/\\n"
        "tag.id=google_apis\\ntarget=android-37.0\\n' > \"$avddir/config.ini\"\n"
        if write_config else ""
    )
    script.write_text(
        "#!/usr/bin/env bash\n"
        f"python3 -c \"import json,sys;print(json.dumps(sys.argv[1:]))\" \"$@\" >> '{log}'\n"
        'target=""\nprev=""\nname=""\n'
        'for a in "$@"; do\n'
        '  [ "$prev" = "--name" ] && name="$a"\n'
        '  [ "$prev" = "--path" ] && target="$a"\n'
        '  prev="$a"\n'
        'done\n'
        'if [ -n "$ANDROID_AVD_HOME" ] && [ -d "$ANDROID_AVD_HOME" ]; then\n'
        '  base="$ANDROID_AVD_HOME"\n'
        'else base="$HOME/.android/avd"; fi\n'
        'if [ -n "$target" ]; then avddir="$target"; else avddir="$base/$name.avd"; fi\n'
        'mkdir -p "$avddir"\n'
        + meta_cfg + data_cfg
    )
    script.chmod(0o755)


def fake_emulator(sdk: Path) -> None:
    """Fake emulator -list-avds: lists metadata `*.ini` names from the
    same must-exist root — incomplete AVDs (no config) still list,
    matching the observed CI behaviour."""
    bin_dir = sdk / "emulator"
    bin_dir.mkdir(parents=True, exist_ok=True)
    script = bin_dir / "emulator"
    script.write_text(
        "#!/usr/bin/env bash\n"
        'if [ -n "$ANDROID_AVD_HOME" ] && [ -d "$ANDROID_AVD_HOME" ]; then\n'
        '  base="$ANDROID_AVD_HOME"\n'
        'else base="$HOME/.android/avd"; fi\n'
        'for f in "$base"/*.ini; do [ -f "$f" ] || continue; basename "$f" .ini; done\n'
    )
    script.chmod(0o755)



class CreateAvdTest(unittest.TestCase):
    """Hermetic fake-binary coverage for create_avd: CLI22 must-exist
    ANDROID_AVD_HOME semantics, explicit --path, metadata/data split,
    and post-creation pin verification. No real SDK/AVD/tool touched."""

    AVD = "agent-mobile-api37"
    TARGET = "android-37.0"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.sdk = Path(self.tmp.name) / "sdk"
        self.home = Path(self.tmp.name) / "home"
        self.home.mkdir(parents=True)
        self.argv_log = Path(self.tmp.name) / "avdmanager-argv.log"
        self.env = {
            "HOME": str(self.home),
            "ANDROID_USER_HOME": None,
            "ANDROID_EMULATOR_HOME": None,
            "ANDROID_SDK_HOME": None,
        }

    def _run(self, env_extra=None):
        env = dict(self.env)
        if env_extra:
            env.update(env_extra)
        return run_setup(self.sdk, "create_avd", env_overrides=env)

    def _seed(self, **kwargs):
        fake_avdmanager(self.sdk, self.argv_log, **kwargs)
        fake_emulator(self.sdk)

    def _meta(self, avd_home: Path) -> Path:
        return avd_home / f"{self.AVD}.ini"

    def _config(self, avd_home: Path) -> Path:
        return avd_home / f"{self.AVD}.avd" / "config.ini"

    def _argv(self) -> list:
        if not self.argv_log.exists():
            return []
        return [json.loads(line) for line in self.argv_log.read_text().splitlines() if line]

    def _valid_config(self) -> str:
        return (
            f"abi.type={host_abi()}\n"
            "hw.device.name=pixel_7\n"
            f"image.sysdir.1=system-images/{self.TARGET}/google_apis/{host_abi()}/\n"
            "tag.id=google_apis\n"
            f"target={self.TARGET}\n"
        )

    def _snapshot(self, root: Path) -> dict:
        return {
            str(p.relative_to(root)): p.read_bytes()
            for p in root.rglob("*") if p.is_file()
        }

    def test_fresh_custom_avd_home_creates_under_configured_path(self):
        custom = self.home / "nested dir" / "avd-root"
        self.assertFalse(custom.exists(), "must-exist lookup requires absence at start")
        self._seed()
        proc = self._run({"ANDROID_AVD_HOME": str(custom)})
        self.assertEqual(0, proc.returncode, proc.stderr + proc.stdout)
        meta = self._meta(custom)
        self.assertTrue(meta.is_file(), "metadata .ini must land in configured root")
        self.assertIn(
            f"path={custom / self.AVD}.avd".encode(), meta.read_bytes(),
            "metadata must point at the requested data dir",
        )
        self.assertIn(b"target=" + self.TARGET.encode(), meta.read_bytes())
        self.assertTrue(self._config(custom).is_file(), "data config must land under --path")
        fallback = self.home / ".android" / "avd"
        self.assertFalse(
            (fallback / f"{self.AVD}.ini").exists() or (fallback / f"{self.AVD}.avd").exists(),
            "no fallback metadata or data may be created",
        )
        calls = self._argv()
        self.assertEqual(1, len(calls), "exactly one avdmanager invocation")
        argv = calls[0]
        self.assertEqual(str(custom / f"{self.AVD}.avd"), argv[argv.index("--path") + 1])
        self.assertNotIn("--force", argv)
        self.assertNotIn("-f", argv)

    def test_existing_valid_avd_short_circuits_without_avdmanager(self):
        custom = self.home / "avds"
        avd = custom / f"{self.AVD}.avd"
        avd.mkdir(parents=True)
        self._meta(custom).write_text(
            f"path={avd}\ntarget={self.TARGET}\n"
        )
        self._config(custom).write_text(self._valid_config())
        self._seed()
        before = self._snapshot(custom)
        proc = self._run({"ANDROID_AVD_HOME": str(custom)})
        self.assertEqual(0, proc.returncode, proc.stderr + proc.stdout)
        self.assertEqual([], self._argv(), "avdmanager must not run for a valid AVD")
        self.assertEqual(before, self._snapshot(custom), "fixture bytes must be unchanged")

    def test_existing_mismatched_avd_fails_without_avdmanager(self):
        custom = self.home / "avds"
        avd = custom / f"{self.AVD}.avd"
        avd.mkdir(parents=True)
        self._meta(custom).write_text(f"path={avd}\ntarget={self.TARGET}\n")
        self._config(custom).write_text("hw.device.name=other\n")
        self._seed()
        before = self._snapshot(custom)
        proc = self._run({"ANDROID_AVD_HOME": str(custom)})
        self.assertNotEqual(0, proc.returncode)
        self.assertEqual([], self._argv(), "avdmanager must not run on mismatch")
        self.assertEqual(before, self._snapshot(custom))

    def test_metadata_only_creation_fails_once(self):
        custom = self.home / "avds"
        self._seed(write_config=False)
        proc = self._run({"ANDROID_AVD_HOME": str(custom)})
        self.assertNotEqual(0, proc.returncode)
        self.assertIn("was not created with the pinned config", proc.stderr + proc.stdout)
        calls = self._argv()
        self.assertEqual(1, len(calls), "no retry")
        self.assertNotIn("--force", calls[0])
        self.assertTrue(self._meta(custom).is_file())
        self.assertFalse(self._config(custom).exists())

    def test_config_only_without_metadata_fails_once(self):
        custom = self.home / "avds"
        self._seed(write_metadata=False)
        proc = self._run({"ANDROID_AVD_HOME": str(custom)})
        self.assertNotEqual(0, proc.returncode)
        self.assertIn("was not created with the pinned config", proc.stderr + proc.stdout)
        self.assertEqual(1, len(self._argv()), "no retry")
        self.assertFalse(self._meta(custom).exists(), "no metadata may be fabricated")
        self.assertTrue(self._config(custom).is_file())

    def test_default_home_avd_root_when_env_absent(self):
        self._seed()
        proc = self._run({"ANDROID_AVD_HOME": None})
        self.assertEqual(0, proc.returncode, proc.stderr + proc.stdout)
        default = self.home / ".android" / "avd"
        self.assertTrue(self._meta(default).is_file())
        self.assertTrue(self._config(default).is_file())
        calls = self._argv()
        self.assertEqual(1, len(calls))
        argv = calls[0]
        self.assertEqual(str(default / f"{self.AVD}.avd"), argv[argv.index("--path") + 1])



if __name__ == "__main__":
    unittest.main()
