"""Measurement flows: per-verb iterations, tree sizes, soak, and scenario."""
import time

from scripts.benchkit.config import TEXT
from scripts.benchkit.core import find


def gesture(b, env, verb, role, name, *extra, tag=None, i=None):
    # Android targets are interactive: a gesture may navigate away, sometimes
    # after its own reply, so re-snapshot first (tagged apart from plain
    # snapshots) and relaunch when the target is gone. iOS targets a static
    # title and skips both.
    if b.conf["resnap"]:
        env = b.run("snapshot", tag="resnap", i=i)
        if not find(env, role, name):
            env = b.run("launch", b.conf["settings"], tag="relaunch", i=i)
    return b.act(env, verb, role, name, *extra, tag=tag, i=i)


def iteration(b, i):
    c = b.conf
    b.run("status", i=i)
    first = env = b.run("launch", c["settings"], app="Settings", i=i)
    env = b.run("snapshot", app="Settings", i=i)
    env = b.act(env, "tap", *c["tap"], i=i)
    env = b.run("back", i=i)
    env = b.run("swipe", "up", i=i)
    env = b.run("swipe", "down", i=i)
    env = gesture(b, env, "doubletap", *c["gesture"], i=i)
    env = gesture(b, env, "hold", *c["gesture"], "--duration", "0.5", i=i)
    env = gesture(b, env, "twofinger", *c["gesture"], i=i)
    env = gesture(b, env, "pinch", c["pinch"], None, "0.6", i=i)
    n = (10, 50, 200)[i % 3]
    if c["open"]:
        # Not gesture(): the open target is meant to vanish, so no relaunch.
        env = b.act(b.run("snapshot", tag="resnap", i=i), "tap", *c["open"], tag="open", i=i)
    gesture(b, env, "type", *c["type"], TEXT[:n], tag=f"type{n}", i=i)
    b.run("screenshot", i=i)
    b.act(first, "tap", *c["stale"], tag="stale", i=i)
    b.run("stop", i=i)
    b.run("home", i=i)
    b.run("center", "notification", i=i)
    b.run("home", i=i)


def apps(b, reps):
    for name, bundle in b.apps.items():
        for _ in range(reps):
            b.run("launch", bundle, tag="launch", app=name)
            b.run("snapshot", tag="tree", app=name)
        b.payload[name] = {"json": len(b.call("snapshot")[2]), "text": len(b.call("snapshot", json_mode=False)[2]),
                           **{f"d{d}": len(b.call("--max-depth", str(d), "snapshot", json_mode=False)[2]) for d in (3, 6)}}


def soak(b, runs):
    b.run("launch", b.conf["settings"], tag="launch", app="Settings")
    for k in range(runs * 3):
        b.run("snapshot", tag="soak", i=k)


def scenario(b):
    c = b.conf
    start, steps = time.perf_counter(), 0
    env = b.run("launch", c["settings"], tag="scenario")
    for role, name in c["drill"]:
        env, steps = b.act(env, "tap", role, name, tag="scenario"), steps + 1
    for _ in range(len(c["drill"])):
        env, steps = b.run("back", tag="scenario"), steps + 1
    if c["open"]:
        env, steps = b.act(env, "tap", *c["open"], tag="scenario"), steps + 1
    role, name, text, check = c["stype"]
    env = b.act(env, "type", role, name, text, tag="scenario")
    ok = check in (env.get("data") or {}).get("text", "")
    b.scenario.append({"ms": round((time.perf_counter() - start) * 1000), "steps": steps + 2, "ok": ok})
