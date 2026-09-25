#!/usr/bin/env python3
"""Benchmark the agent-mobile CLI against a live simulator and write an HTML report.

    python3 scripts/bench.py                      # full run, about 25 min
    python3 scripts/bench.py --quick              # smoke run, about 5 min
    python3 scripts/bench.py --compare old.json   # show deltas against an earlier run
    python3 scripts/bench.py --render run.json    # rebuild the HTML only

Results land in bench-results/<stamp>.json and .html. Stdlib only.
"""
import argparse
import json
import os
import re
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "bench-results"
STATE = Path.home() / ".agent-mobile" / "state.json"
SETTINGS = "com.apple.Preferences"
APPS = {"Settings": SETTINGS, "Calendar": "com.apple.mobilecal", "Contacts": "com.apple.MobileAddressBook",
        "Photos": "com.apple.mobileslideshow", "Maps": "com.apple.Maps"}
TEXT = "the quick brown fox jumps over the lazy dog " * 5


def sh(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()


class Bench:
    def __init__(self, binary, device, pinned):
        self.bin, self.device, self.t0, self.pid = binary, device, time.time(), None
        self.flags = ["--device", device] if pinned else []
        self.samples, self.cold, self.scenario, self.payload = [], [], [], {}

    def runner(self):
        # ponytail: the in-sim XCTest runner is a host process; state.json only records xcodebuild.
        if not self.pid or not sh("ps", "-o", "pid=", "-p", self.pid):
            self.pid = (sh("pgrep", "-f", "AgentMobileDriver-Runner.app/AgentMobileDriver-Runner").split() or [None])[0]
        out = sh("ps", "-o", "rss=,time=", "-p", self.pid).split() if self.pid else []
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


def stop_driver(device):
    entry = json.loads(STATE.read_text()).get("devices", {}).get(device) if STATE.exists() else None
    if not entry:
        return
    try:
        os.kill(entry["pid"], signal.SIGTERM)
    except ProcessLookupError:
        return
    for _ in range(60):
        if subprocess.run(["kill", "-0", str(entry["pid"])], capture_output=True).returncode:
            return
        time.sleep(0.5)


def cold(b, udid):
    stop_driver(b.device)
    sh("xcrun", "simctl", "shutdown", udid)
    start = time.perf_counter()
    sh("xcrun", "simctl", "boot", udid)
    sh("xcrun", "simctl", "bootstatus", udid, "-b")
    boot = (time.perf_counter() - start) * 1000
    driver = b.call("status")[0]
    snap = b.call("snapshot")[0]
    b.cold.append({"boot": round(boot), "driver": round(driver), "snapshot": round(snap)})


def iteration(b, i):
    b.run("status", i=i)
    first = env = b.run("launch", SETTINGS, app="Settings", i=i)
    env = b.run("snapshot", app="Settings", i=i)
    env = b.act(env, "tap", "button", "General", i=i)
    env = b.run("back", i=i)
    env = b.run("swipe", "up", i=i)
    env = b.run("swipe", "down", i=i)
    env = b.act(env, "doubletap", "text", "Settings", i=i)
    env = b.act(env, "hold", "text", "Settings", "--duration", "0.5", i=i)
    env = b.act(env, "twofinger", "text", "Settings", i=i)
    env = b.act(env, "pinch", "collectionview", None, "0.6", i=i)
    n = (10, 50, 200)[i % 3]
    b.act(env, "type", "searchfield", None, TEXT[:n], tag=f"type{n}", i=i)
    b.run("screenshot", i=i)
    b.act(first, "tap", "button", "General", tag="stale", i=i)
    b.run("stop", i=i)
    b.run("home", i=i)
    b.run("center", "notification", i=i)
    b.run("home", i=i)


def apps(b, reps):
    for name, bundle in APPS.items():
        for _ in range(reps):
            b.run("launch", bundle, tag="launch", app=name)
            b.run("snapshot", tag="tree", app=name)
        b.payload[name] = {"json": len(b.call("snapshot")[2]), "text": len(b.call("snapshot", json_mode=False)[2]),
                           **{f"d{d}": len(b.call("--max-depth", str(d), "snapshot", json_mode=False)[2]) for d in (3, 6)}}


def scenario(b):
    start, steps = time.perf_counter(), 0
    env = b.run("launch", SETTINGS, tag="scenario")
    for role, name in (("button", "General"), ("button", "About")):
        env, steps = b.act(env, "tap", role, name, tag="scenario"), steps + 1
    for _ in range(2):
        env, steps = b.run("back", tag="scenario"), steps + 1
    env = b.act(env, "type", "searchfield", None, "Bluetooth", tag="scenario")
    ok = "Bluetooth" in (env.get("data") or {}).get("text", "")
    b.scenario.append({"ms": round((time.perf_counter() - start) * 1000), "steps": steps + 2, "ok": ok})


def meta(b, runs, udid):
    dev = next((d for d in json.loads(b.call("devices")[2] or b"{}").get("devices", []) if d["udid"] == udid), {})
    return {"stamp": time.strftime("%Y-%m-%d %H:%M"), "runs": runs, "device": b.device, "os": dev.get("os"),
            "sha": sh("git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"),
            "dirty": bool(sh("git", "-C", str(ROOT), "status", "--porcelain")),
            "cli": sh(b.bin, "--version"), "xcode": sh("xcodebuild", "-version").replace("\n", " ")}


def render(run, base, path):
    data = f"const RUN = {json.dumps(run)};\nconst BASE = {json.dumps(base)};"
    html = (ROOT / "scripts" / "bench.html").read_text().replace("/*DATA*/", data)
    path.write_text(html)
    print(f"report: {path}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runs", type=int, default=30)
    ap.add_argument("--cold", type=int, default=3, help="cold boots; 0 skips")
    ap.add_argument("--quick", action="store_true", help="5 runs, 1 cold boot")
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "agent-mobile"))
    ap.add_argument("--device", help="default: the CLI's remembered device")
    ap.add_argument("--compare", type=Path, help="earlier results JSON")
    ap.add_argument("--render", type=Path, help="rebuild the HTML from a results JSON, run nothing")
    a = ap.parse_args()
    base = json.loads(a.compare.read_text()) if a.compare else None
    if a.render:
        return render(json.loads(a.render.read_text()), base, a.render.with_suffix(".html"))
    if not Path(a.bin).exists():
        raise SystemExit(f"no binary at {a.bin}; run `cargo build --release --locked` first")
    runs, colds = (5, 1) if a.quick else (a.runs, a.cold)
    device = a.device or (json.loads(STATE.read_text()).get("default_device") if STATE.exists() else None)
    b = Bench(a.bin, device, bool(a.device))
    udid = next((d["udid"] for d in json.loads(b.call("devices")[2])["devices"] if d["name"] == device), None)
    if not udid:
        raise SystemExit(f"no simulator named {device!r}; pass --device <name> (see `agent-mobile devices`)")
    for n in range(colds):
        print(f"cold boot {n + 1}/{colds}", flush=True)
        cold(b, udid)
    b.call("status")
    for i in range(runs):
        print(f"iteration {i + 1}/{runs}", flush=True)
        iteration(b, i)
    print("tree sizes", flush=True)
    apps(b, max(2, runs // 5))
    print("soak", flush=True)
    b.run("launch", SETTINGS, tag="launch", app="Settings")
    for k in range(runs * 3):
        b.run("snapshot", tag="soak", i=k)
    for n in range(max(3, runs // 3)):
        print(f"scenario {n + 1}", flush=True)
        scenario(b)
    run = {"meta": meta(b, runs, udid), "cold": b.cold, "samples": b.samples,
           "scenario": b.scenario, "payload": b.payload}
    OUT.mkdir(exist_ok=True)
    path = OUT / time.strftime("%Y%m%d-%H%M%S.json")
    path.write_text(json.dumps(run))
    print(f"results: {path}")
    render(run, base, path.with_suffix(".html"))


if __name__ == "__main__":
    main()
