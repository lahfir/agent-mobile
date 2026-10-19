#!/usr/bin/env python3
"""Time one agent session over MCP stdio, the way an agent client sees it.

Per platform: release any session, then `launch` Settings with the device
key (session warm-up on a booted device), then a fixed list of actions.
Prints one JSON object: per-platform medians plus gates. Drives the
devices only through agent-mobile.

    python3 scripts/bench_agent_loop.py [--bin npm/bin/agent-mobile]
        [--platform ios|android] [--smoke] [--ios-key KEY] [--android-key KEY]

Keys come from `agent-mobile devices`; the defaults are the run's own devices.
"""
import argparse
import json
import re
import statistics
import subprocess
import sys
import time

KEYS = {"ios": "ios:9B7CEE9E-DED1-4A84-8ABD-E4A381E4CFDD", "android": "android:avd:agent-mobile-api37"}
NEVER = "Nabihah"

# (tool, args, ref-finder (role, name) or None, name that must be in the reply)
STEPS = {
    "ios": [
        ("snapshot", {}, None, "General"),
        ("tap", {}, ("button", "General"), "About"),
        ("tap", {}, ("button", "About"), "iOS Version"),
        ("back", {}, None, "About"),
        ("back", {}, None, "General"),
        ("swipe", {"direction": "up"}, None, None),
        ("swipe", {"direction": "down"}, None, "General"),
        ("type", {"text": "Bluetooth"}, ("searchfield", None), "Bluetooth"),
        ("home", {}, None, None),
    ],
    "android": [
        ("snapshot", {}, None, "Apps"),
        ("tap", {}, ("text", "Apps"), "Default apps"),
        ("back", {}, None, "Apps"),
        ("swipe", {"direction": "up"}, None, None),
        ("swipe", {"direction": "down"}, None, "Apps"),
        ("tap", {}, ("text", "Search Settings"), "Search settings"),
        ("type", {"text": "Bluetooth"}, ("textfield", None), "Bluetooth"),
        ("home", {}, None, None),
    ],
}
APP = {"ios": "com.apple.Preferences", "android": "com.android.settings"}


class Mcp:
    def __init__(self, binary):
        self.p = subprocess.Popen([binary, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.n = 0
        init = self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                       "clientInfo": {"name": "bench", "version": "0"}})
        if "result" not in init:
            raise SystemExit(f"MCP initialize failed: {init.get('error')}")
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")

    def rpc(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        while True:
            msg = json.loads(self.p.stdout.readline())
            if msg.get("id") == self.n:
                return msg

    def call(self, name, args):
        start = time.perf_counter()
        msg = self.rpc("tools/call", {"name": name, "arguments": args})
        ms = (time.perf_counter() - start) * 1000
        res = msg.get("result")
        if not isinstance(res, dict) or not isinstance(res.get("content"), list):
            # A JSON-RPC error or a malformed result is a failed step, never a pass.
            return ms, {"isError": True}, f"protocol error: {msg.get('error') or msg}"
        text = "".join(c.get("text", "") for c in res["content"])
        return ms, res, text

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=30)


def find(text, role, name):
    pat = rf'(@\S+) {role} "{re.escape(name)}"' if name else rf"(@\S+) {role} "
    m = re.search(pat, text)
    return m.group(1) if m else None


def run_platform(mcp, platform, smoke):
    dev = KEYS[platform]
    assert NEVER not in dev
    mcp.call("release", {"device": dev})
    warm_ms, res, text = mcp.call("launch", {"app_id": APP[platform], "device": dev})
    out = {"warmup_ms": warm_ms, "ok": not res.get("isError"), "stale": 0, "found": True, "late": 0,
           "steps": [{"tool": "launch", "ms": round(warm_ms), "bytes": len(text)}], "fail": None}
    if res.get("isError"):
        out["fail"] = text[:300]
        return out
    for tool, args, target, expect in ([] if smoke else STEPS[platform]):
        args = {**args, "device": dev}
        if target:
            ref = find(text, *target)
            if not ref:
                out["found"], out["fail"] = False, f"no {target} before {tool}"
                break
            args["target" if tool == "tap" else "ref"] = {"ref": ref} if tool == "tap" else ref
        ms, res, text = mcp.call(tool, args)
        sc = res.get("structuredContent") or {}
        out["steps"].append({"tool": tool, "ms": round(ms), "bytes": len(text), "settle": sc.get("settle_ms"),
                             "reads": sc.get("reads"), "refs": sc.get("ref_count")})
        if "STALE_REF" in text:
            out["stale"] += 1
        if res.get("isError"):
            out["ok"], out["fail"] = False, f"{tool}: {text[:300]}"
            break
        if expect and f'"{expect}' not in text:
            # The reply came before the screen changed; an agent snapshots again.
            out["late"] += 1
            extra, res, text = mcp.call("snapshot", {"device": dev})
            out["steps"][-1]["ms"] += round(extra)
        if expect and f'"{expect}' not in text:
            out["found"], out["fail"] = False, f"{expect!r} missing after {tool}"
            break
    return out


def summary(platform, r):
    acts = r["steps"][1:]
    med = lambda k: statistics.median([s[k] for s in acts if s.get(k) is not None] or [0])
    return {f"{platform}_action_ms": round(statistics.mean([s["ms"] for s in acts] or [0]), 1),
            f"{platform}_warmup_ms": round(r["warmup_ms"]), f"{platform}_settle_ms": med("settle"),
            f"{platform}_reads": med("reads"), f"{platform}_bytes": med("bytes")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="npm/bin/agent-mobile")
    ap.add_argument("--platform", choices=["ios", "android"], action="append")
    ap.add_argument("--smoke", action="store_true")
    ap.add_argument("--ios-key", default=KEYS["ios"])
    ap.add_argument("--android-key", default=KEYS["android"])
    a = ap.parse_args()
    KEYS.update(ios=a.ios_key, android=a.android_key)
    mcp, result, worst = Mcp(a.bin), {}, {}
    ok, found, stale, late = True, True, 0, 0
    for platform in a.platform or ["ios", "android"]:
        r = run_platform(mcp, platform, a.smoke)
        mcp.call("release", {"device": KEYS[platform]})
        result.update(summary(platform, r))
        ok, found, stale, late = ok and r["ok"], found and r["found"], stale + r["stale"], late + r["late"]
        worst[platform] = {"fail": r["fail"], "steps": r["steps"]}
    mcp.close()
    sizes = [v for k, v in result.items() if k.endswith("_bytes")]
    result.update({"steps_ok": int(ok), "stale_refs": stale, "targets_found": int(found), "late_settles": late,
                   "reply_bytes": round(statistics.mean(sizes)) if sizes else 0, "worst_cases": worst})
    json.dump(result, sys.stdout)
    print()


if __name__ == "__main__":
    main()
