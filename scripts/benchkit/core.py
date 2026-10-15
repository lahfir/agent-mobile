"""Benchmark session: run CLI verbs, sample timings plus host resources."""
import json
import re
import subprocess
import time

from scripts.benchkit.config import APPS, CONF, STATE
from scripts.benchkit.select import sh


class Bench:
    def __init__(self, binary, device, pinned, platform="ios"):
        self.bin, self.device, self.t0, self.pid = binary, device, time.time(), None
        self.flags = ["--device", device] if pinned else []
        self.samples, self.cold, self.scenario, self.payload = [], [], [], {}
        self.platform, self.conf, self.apps = platform, CONF[platform], APPS[platform]
        self.avd, self.adb, self.emulator, self.selected_kind, self.apk = None, None, None, None, None

    def runner(self):
        if self.platform == "android":
            return self.serve_stats()
        # ponytail: the in-sim XCTest runner is a host process; state.json only records xcodebuild.
        if not self.pid or not sh("ps", "-o", "pid=", "-p", self.pid):
            self.pid = (sh("pgrep", "-f", "AgentMobileDriver-Runner.app/AgentMobileDriver-Runner").split() or [None])[0]
        out = sh("ps", "-o", "rss=,time=", "-p", self.pid).split() if self.pid else []
        if len(out) < 2:
            return None, None
        secs = sum(float(p) * 60 ** i for i, p in enumerate(reversed(out[1].split(":"))))
        return int(out[0]) // 1024, round(secs, 2)

    def serve_stats(self):
        # Android: the bridge lives in the serve host process, whose pid the
        # state file records. Re-read every sample: cold boots restart serve.
        pid = None
        if STATE.exists():
            pid = json.loads(STATE.read_text()).get("devices", {}).get(self.device, {}).get("pid")
        out = sh("ps", "-o", "rss=,time=", "-p", str(pid)).split() if pid else []
        if len(out) < 2:
            return None, None
        secs = sum(float(p) * 60 ** i for i, p in enumerate(reversed(out[1].split(":"))))
        return int(out[0]) // 1024, round(secs, 2)

    def call(self, *args, json_mode=True, timeout=180):
        cmd = [self.bin, *(["--json"] if json_mode else []), *self.flags, *args]
        start = time.perf_counter()
        try:
            p = subprocess.run(cmd, capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return (time.perf_counter() - start) * 1000, 124, b"", b"timeout"
        return (time.perf_counter() - start) * 1000, p.returncode, p.stdout, p.stderr

    def run(self, *args, tag=None, app=None, i=None):
        wall, rc, out, err = self.call(*args)
        try:
            env = json.loads(out)
        except ValueError:
            env = {}
        d = env.get("data") or {}
        rss, cpu = self.runner()
        code = (env.get("error") or {}).get("code") or (err.decode(errors="replace").strip()[-160:] if rc else None)
        self.samples.append({
            "v": tag or args[0], "app": app, "i": i, "wall": round(wall, 1), "ms": env.get("elapsed_ms"),
            "settle": d.get("settle_ms"), "reads": d.get("reads"), "settled": d.get("settled"),
            "refs": d.get("ref_count"), "bytes": len(d.get("png_base64", "")) * 3 // 4 or len(out),
            "ok": rc == 0, "err": code, "rss": rss, "cpu": cpu, "t": round(time.time() - self.t0, 2)})
        return env

    def act(self, env, verb, role, name, *extra, tag=None, i=None):
        ref = find(env, role, name)
        if not ref:
            self.samples.append({"v": tag or verb, "i": i, "ok": False, "err": f"NO_TARGET {role} {name or ''}"})
            return env
        return self.run(verb, ref, *extra, tag=tag, i=i)


def find(env, role, name=None):
    text = (env.get("data") or {}).get("text", "")
    pat = rf'(@\S+) {role} "{re.escape(name)}"' if name else rf"(@\S+) {role} "
    m = re.search(pat, text)
    return m.group(1) if m else None
