"""Driver lifecycle for cold boots: stop the recorded serve, reap orphans."""
import contextlib
import json
import os
import signal
import subprocess
import time

from scripts.benchkit.config import STATE


def stop_driver(device, state_path=None, term_grace=30):
    state_file = state_path or STATE
    entry = json.loads(state_file.read_text()).get("devices", {}).get(device) if state_file.exists() else None
    if not entry:
        return
    try:
        os.kill(entry["pid"], signal.SIGTERM)
    except ProcessLookupError:
        return
    if _wait_gone(entry["pid"], term_grace):
        return
    with contextlib.suppress(ProcessLookupError):
        os.kill(entry["pid"], signal.SIGKILL)
    if not _wait_gone(entry["pid"], 5):
        raise SystemExit(f"driver pid {entry['pid']} refuses to die; aborting before it poisons the run")


def _wait_gone(pid, seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        except PermissionError:
            return False
        time.sleep(0.5)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return True
    return False


def _sweep_serves(device):
    # A recorded pid is one serve; races can orphan others on the same
    # device, and two bridges on one emulator poison every sample. Reap
    # them all by command line before a cold boot starts competing work.
    out = subprocess.run(["ps", "-axo", "pid=,args="], capture_output=True, text=True).stdout
    for line in out.splitlines():
        parts = line.split(None, 1)
        if len(parts) < 2 or f"agent-mobile serve {device}" not in parts[1]:
            continue
        pid = int(parts[0])
        with contextlib.suppress(ProcessLookupError):
            os.kill(pid, signal.SIGTERM)
        if _wait_gone(pid, 10):
            continue
        with contextlib.suppress(ProcessLookupError):
            os.kill(pid, signal.SIGKILL)
        if not _wait_gone(pid, 5):
            raise SystemExit(f"orphan serve pid {pid} refuses to die; aborting")
