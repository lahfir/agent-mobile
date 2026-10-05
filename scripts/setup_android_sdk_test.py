"""Hermetic tests for setup-android-sdk.sh revision pinning and
run_with_deadline.py group cleanup. Everything is temp-dir/fake-binary —
no real SDK, adb, or emulator is touched.
"""

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


def run_setup(home: Path, body: str) -> subprocess.CompletedProcess:
    env = dict(os.environ, ANDROID_HOME=str(home))
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

if __name__ == "__main__":
    unittest.main()
