#!/usr/bin/env python3
"""Signal-safe deadline wrapper: run argv under a seconds budget.

Signals are blocked with pthread_sigmask while handlers install, so no
signal can land in the window between mask setup and Popen. Timeout ->
SIGTERM the child's process group, wait up to 5s for the group to
disappear (a child exit alone is not enough), SIGKILL, reap, exit 124.
SIGINT/SIGTERM/SIGHUP on the wrapper -> same group cleanup, exit
128+signum. BaseException after spawn -> cleanup, re-raise. Normal exit
passes the child status through untouched. Logs nothing.
"""

import os
import signal
import subprocess
import sys
import time

WATCHED = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
GRACE_S = 5.0


class _Caught(BaseException):
    """Received signal number, raised from inside a handler."""


def _group_alive(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _reap_group(proc: subprocess.Popen) -> None:
    """TERM the whole group, bound the grace, KILL, reap the child.

    Never returns just because the direct child exited — a descendant
    that ignored TERM must not survive.
    """
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        pass
    deadline = time.monotonic() + GRACE_S
    group_alive = _group_alive(proc.pid)
    while group_alive and time.monotonic() < deadline:
        proc.poll()
        time.sleep(0.05)
        group_alive = _group_alive(proc.pid)
    if group_alive:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


def main() -> int:
    if len(sys.argv) < 3:
        return 2
    try:
        secs = float(sys.argv[1])
    except ValueError:
        return 2
    if secs != secs or secs <= 0 or secs == float("inf"):
        return 2

    proc = None
    caught = []
    cleaning = False

    def handler(signum, _frame):
        nonlocal cleaning
        if proc is None:
            caught.append(signum)
            return
        if cleaning:
            return
        raise _Caught(signum)

    old_mask = signal.pthread_sigmask(signal.SIG_BLOCK, WATCHED)
    orig = {sig: signal.signal(sig, handler) for sig in WATCHED}
    signal.pthread_sigmask(signal.SIG_SETMASK, old_mask)
    try:
        proc = subprocess.Popen(sys.argv[2:], start_new_session=True)
        if caught:
            _reap_group(proc)
            return 128 + caught[0]
        return proc.wait(timeout=secs)
    except subprocess.TimeoutExpired:
        cleaning = True
        _reap_group(proc)
        return 124
    except _Caught as e:
        cleaning = True
        _reap_group(proc)
        return 128 + int(e.args[0])
    except BaseException:
        cleaning = True
        if proc is not None:
            _reap_group(proc)
        raise
    finally:
        signal.pthread_sigmask(signal.SIG_BLOCK, WATCHED)
        for sig, prev in orig.items():
            signal.signal(sig, prev)
        signal.pthread_sigmask(signal.SIG_SETMASK, old_mask)


if __name__ == "__main__":
    sys.exit(main())
