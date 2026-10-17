"""Benchmark the agent-mobile CLI against a live simulator or emulator and write an HTML report.

    python3 scripts/bench.py                      # full run, about 25 min
    python3 scripts/bench.py --quick              # smoke run, about 5 min
    python3 scripts/bench.py --device android:avd:<name>   # Android instead of iOS
    python3 scripts/bench.py --compare old.json   # show deltas against an earlier run
    python3 scripts/bench.py --render run.json    # rebuild the HTML only

Results land in bench-results/<stamp>.json and .html. Stdlib only.
"""
import argparse
import json
import time
from pathlib import Path

from scripts.benchkit.cold import android_sdk, cold, driver_apk
from scripts.benchkit.config import OUT, ROOT, STATE
from scripts.benchkit.core import Bench
from scripts.benchkit.flows import apps, iteration, scenario, soak
from scripts.benchkit.report import meta, render
from scripts.benchkit.select import bench_target, canonical_bench_selection, select_device


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runs", type=int, default=30)
    ap.add_argument("--cold", type=int, default=3, help="cold boots; 0 skips")
    ap.add_argument("--quick", action="store_true", help="5 runs, 1 cold boot")
    ap.add_argument("--bin", default=str(ROOT / "target" / "release" / "agent-mobile"))
    ap.add_argument("--device", help="default: the CLI's remembered device")
    ap.add_argument("--compare", type=Path, help="earlier results JSON")
    ap.add_argument("--render", type=Path, help="rebuild the HTML from a results JSON, run nothing")
    ap.add_argument("--agent", action="store_true", help="run agent tasks through MCP")
    ap.add_argument("--servers", type=Path, help="local JSON map of comparison servers")
    ap.add_argument("--tasks", help="agent task ids, comma separated (default: all)")
    ap.add_argument("--seeds", type=int, default=2, help="agent seed variants per task")
    ap.add_argument("--platforms", help="agent platforms, comma separated (default: ios,android)")
    a = ap.parse_args()
    if a.agent:
        from scripts.benchkit.agent import run_agent
        return run_agent(a.servers, a.bin, a.tasks, a.seeds, a.platforms, a.device)
    base = json.loads(a.compare.read_text()) if a.compare else None
    if a.render:
        return render(json.loads(a.render.read_text()), base, a.render.with_suffix(".html"))
    if not Path(a.bin).exists():
        raise SystemExit(f"no binary at {a.bin}; run `cargo build --release --locked` first")
    runs, colds = (5, 1) if a.quick else (a.runs, a.cold)
    device = a.device or (json.loads(STATE.read_text()).get("default_device") if STATE.exists() else None)
    b = Bench(a.bin, device, bool(a.device))
    devices = json.loads(b.call("devices")[2])["devices"]
    selected = select_device(devices, device)
    platform, match, boot = bench_target(selected, device)
    device, pinned = canonical_bench_selection(selected, device)
    b = Bench(a.bin, device, pinned, platform)
    b.selected_kind = (selected or {}).get("kind")
    if platform == "android":
        b.avd = boot
        b.adb, b.emulator = android_sdk()
        b.apk = driver_apk()
    for n in range(colds):
        print(f"cold boot {n + 1}/{colds}", flush=True)
        cold(b, boot)
    b.call("status")
    if platform == "android":
        warmup(b)
    for i in range(runs):
        print(f"iteration {i + 1}/{runs}", flush=True)
        iteration(b, i)
    print("tree sizes", flush=True)
    apps(b, max(2, runs // 5))
    print("soak", flush=True)
    soak(b, runs)
    for n in range(max(3, runs // 3)):
        print(f"scenario {n + 1}", flush=True)
        scenario(b)
    run = {"meta": meta(b, runs, match), "cold": b.cold, "samples": b.samples,
           "scenario": b.scenario, "payload": b.payload}
    OUT.mkdir(exist_ok=True)
    path = OUT / time.strftime("%Y%m%d-%H%M%S.json")
    path.write_text(json.dumps(run))
    print(f"results: {path}")
    render(run, base, path.with_suffix(".html"))


def warmup(b):
    # A freshly booted emulator settles for a while (dexopt, GMS init):
    # iterations must measure steady state, so block until snapshots pass.
    print("warmup", flush=True)
    deadline, last = time.time() + 120, "no attempt yet"
    while True:
        ms, rc, out, err = b.call("snapshot")
        if rc == 0:
            break
        try:
            e = json.loads(out).get("error", {})
            last = f"{e.get('code')}: {e.get('message')}"
        except (ValueError, AttributeError):
            last = (err or out or b"")[:160].decode("utf-8", "replace")
        if time.time() > deadline:
            raise SystemExit(f"emulator never reached steady state; last snapshot: {last}")
        time.sleep(5)
