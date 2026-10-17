"""Run fixed agent tasks through a single MCP server per run."""
import json
import os
import pathlib
import signal
import subprocess
import tempfile
import time

from scripts.benchkit.cold import android_sdk, android_serial
from scripts.benchkit.config import OUT
from scripts.benchkit.tasks import TASKS, task_variant

CLAUDE_MODEL = "claude-sonnet-5-5"
TIMEOUT = 180
WARMUP_GOAL = "Take one screenshot of the device, then stop."
AGENT_BINARY = pathlib.Path(__file__).resolve().parents[2] / "target" / "release" / "agent-mobile"


def parse_stream(lines, server):
    steps, stale, final, failed = 0, 0, None, []
    for line in lines:
        try:
            event = json.loads(line)
        except (ValueError, TypeError):
            continue
        data = event.get("data", event)
        message = data.get("message", {}) if isinstance(data, dict) else {}
        if data.get("type") == "system" and data.get("subtype") == "init":
            states = {x.get("name"): x.get("status") for x in data.get("mcp_servers", [])}
            failed = [server] if states.get(server, "failed") == "failed" else []
        content = message.get("content", []) if isinstance(message, dict) else []
        for item in content:
            if item.get("type") == "tool_use":
                steps += 1
            if item.get("type") == "tool_result" and server == "agent-mobile" and "STALE_REF" in str(item):
                stale += 1
        if data.get("type") == "result" or event.get("type") == "result":
            final = data
    return {"steps": steps, "stale_refs": stale if server == "agent-mobile" else None, "final": final,
            "failed": failed}


def server_map(path, binary=AGENT_BINARY, device=None):
    args = ["--device", device, "mcp"] if device else ["mcp"]
    servers = {"agent-mobile": {"command": str(pathlib.Path(binary).absolute()), "args": args, "env": {}}}
    if path is None:
        return servers
    extra = json.loads(path.read_text())
    if not isinstance(extra, dict) or any(not isinstance(k, str) or not isinstance(v, dict) or
                                          not isinstance(v.get("command"), str) or
                                          not isinstance(v.get("args", []), list) or
                                          not isinstance(v.get("env", {}), dict) or
                                          not isinstance(v.get("disallowedTools", []), list) or
                                          not all(isinstance(x, str) for x in v.get("disallowedTools", [])) or
                                          not isinstance(v.get("teardown", []), list) or
                                          not all(isinstance(c, list) and all(isinstance(x, str) for x in c)
                                                  for c in v.get("teardown", []))
                                          for k, v in extra.items()):
        raise ValueError("servers file must map labels to command, args, and env")
    if "agent-mobile" in extra:
        raise ValueError("agent-mobile is a built-in server")
    return {**servers, **extra}


def keep_streams(keep, stdout, stderr):
    if keep:
        keep = pathlib.Path(keep)
        keep.parent.mkdir(parents=True, exist_ok=True)
        keep.with_name(keep.name + ".jsonl").write_text(stdout or "")
        keep.with_name(keep.name + ".err").write_text(stderr or "")


def run_claude(goal, label, server, cwd, timeout=TIMEOUT, keep=None):
    with tempfile.TemporaryDirectory() as config_dir:
        config = pathlib.Path(config_dir) / "mcp.json"
        disallowed = server.get("disallowedTools", [])
        config_server = {key: value for key, value in server.items() if key not in ("disallowedTools", "teardown")}
        config.write_text(json.dumps({"mcpServers": {label: config_server}}))
        config.chmod(0o600)
        env = os.environ.copy()
        env["ENABLE_TOOL_SEARCH"] = "false"
        start = time.monotonic()
        denied = ["--disallowedTools", ",".join(f"mcp__{label}__{name}" for name in disallowed)] if disallowed else []
        command = ["claude", "-p", "--setting-sources", "", "--disable-slash-commands", "--model", CLAUDE_MODEL, "--mcp-config", str(config),
                   "--strict-mcp-config", "--tools", "", "--allowedTools", f"mcp__{label}", *denied,
                   "--no-session-persistence", "--output-format", "stream-json", "--verbose", goal]
        proc = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, start_new_session=True)
        try:
            stdout, stderr = proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            os.killpg(proc.pid, signal.SIGKILL)
            stdout, stderr = proc.communicate()
            keep_streams(keep, stdout, stderr)
            out = stdout or (exc.stdout.decode() if isinstance(exc.stdout, bytes) else exc.stdout) or ""
            parsed = parse_stream(out.splitlines(), label)
            if parsed["failed"]:
                return {"status": "invalid", "cause": "server failed to start: " + ", ".join(parsed["failed"]),
                        "wall_ms": timeout * 1000, **parsed}
            return {"status": "success_false", "reason": "timeout", "wall_ms": timeout * 1000, **parsed}
        elapsed = (time.monotonic() - start) * 1000
        keep_streams(keep, stdout, stderr)
        parsed = parse_stream(stdout.splitlines(), label)
        final = parsed["final"]
        cause = ("server failed to start: " + ", ".join(parsed["failed"]) if parsed["failed"] else
                 "claude CLI error" if proc.returncode or not final else
                 f"model error: {final.get('result')}" if final.get("is_error") else None)
        if cause:
            return {"status": "invalid", "cause": cause, "wall_ms": elapsed, **parsed}
        return {"status": "agent_finished", "reason": final.get("result"), "wall_ms": elapsed, **parsed}




def fill(command, values, device):
    fmt = {key: str(value) for key, value in values.items()}
    fmt["device"] = device.rsplit(":", 1)[-1] if device else ""
    argv = [part.format(**fmt) for part in command]
    if argv[0] != "adb":
        return argv
    adb = android_sdk()[0]
    key = device.split(":", 1)[1] if device and device.startswith("android:") else ""
    serial = android_serial(adb, key[4:]) if key.startswith("avd:") else key or None
    if not serial:
        raise OSError(f"could not resolve Android device serial for {device}")
    return [adb, "-s", serial, *argv[1:]]


def reset_device(task, platform, values, device, run=subprocess.run):
    """Run the task's reset commands; return the failure text, or None."""
    try:
        for command in task["reset"][platform]:
            run(fill(command, values, device), capture_output=True, text=True, timeout=60, check=True)
    except (OSError, subprocess.SubprocessError) as exc:
        return str(exc)


def teardown_all(servers, device, run=subprocess.run):
    """Run every server's teardown commands so no server leaves a process behind. Never fails a run."""
    for server in servers.values():
        for command in server.get("teardown", []):
            try:
                run(fill(command, {}, device), capture_output=True, text=True, timeout=30, check=False)
            except (OSError, subprocess.SubprocessError):
                pass


def host_check(check, values, device=None, run=subprocess.run):
    try:
        result = run(fill(check["command"], values, device), capture_output=True, text=True, timeout=30, check=True)
    except (OSError, subprocess.SubprocessError) as exc:
        return None, str(exc)
    evidence = (result.stdout or "").strip()
    return evidence == check["expected"].format(**values), evidence or (result.stderr or "").strip()


def checked_outcome(outcome, check, values, device=None, run=subprocess.run):
    if outcome["status"] != "agent_finished":
        return outcome
    passed, evidence = host_check(check, values, device, run)
    if passed is None:
        return {**outcome, "status": "invalid", "cause": evidence}
    return {**outcome, "status": "success" if passed else "success_false",
            "reason": "host check passed" if passed else evidence}


def csv(text):
    return [part for part in (text or "").split(",") if part]


def run_plan(tasks, seeds, platforms):
    """(platform, task, seed) triples; fails fast on an unknown task or platform."""
    known = {task["id"]: task for task in TASKS}
    if bad := [name for name in csv(tasks) if name not in known]:
        raise SystemExit(f"unknown tasks: {', '.join(bad)}")
    if bad := [name for name in csv(platforms) if name not in ("ios", "android")]:
        raise SystemExit(f"unknown platforms: {', '.join(bad)}")
    chosen = [known[name] for name in csv(tasks)] or TASKS
    return [(platform, task, seed) for platform in csv(platforms) or ["ios", "android"] for task in chosen
            if platform in task["checks"] for seed in range(seeds)]


def pick_device(binary, platform, pinned=None):
    out = subprocess.run([str(binary), "--json", "devices"], capture_output=True, text=True).stdout
    rows = [d for d in json.loads(out or "{}").get("devices", []) if d.get("platform") == platform
            and d.get("kind") in ("simulator", "emulator") and pinned in (None, d.get("key"), d.get("id"), d.get("name"))]
    rows.sort(key=lambda d: d.get("state") not in ("Booted", "device"))
    return rows[0] if rows else None


def run_one(label, server, task, platform, seed, device, streams=None, servers=None):
    """One measured run: teardown, reset, agent, host check. A harness fault is invalid and re-runs once."""
    goal, values = task_variant(task, seed)
    row = {"task": task["id"], "platform": platform, "seed": seed, "server": label}
    invalid_causes = []
    for attempt in (1, 2):
        if servers is not None:
            teardown_all(servers, device)
        fault = reset_device(task, platform, values, device)
        if fault:
            outcome = {"status": "invalid", "cause": f"reset failed: {fault}"}
        else:
            with tempfile.TemporaryDirectory() as cwd:
                keep = streams / f"{platform}-{label}-{task['id']}-s{seed}-a{attempt}" if streams else None
                outcome = run_claude(goal, label, server, cwd, keep=keep)
            outcome = checked_outcome(outcome, task["checks"][platform], values, device)
        outcome.pop("final", None)
        outcome.pop("failed", None)
        if outcome["status"] == "invalid":
            invalid_causes.append(outcome.get("cause", "invalid attempt"))
        if outcome["status"] != "invalid":
            break
    return {**row, **outcome, "attempts": attempt, "invalid_attempts": len(invalid_causes),
            "invalid_causes": invalid_causes}


def run_agent(servers_path, binary=AGENT_BINARY, tasks=None, seeds=2, platforms=None, device=None):
    try:
        servers = server_map(servers_path, binary)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        raise SystemExit(f"invalid servers file: {exc}")
    plan = run_plan(tasks, seeds, platforms)
    version = subprocess.run(["claude", "--version"], capture_output=True, text=True, check=True).stdout.strip()
    rows, warmup = [], {}
    stamp = time.strftime("%Y%m%d-%H%M%S")
    streams = OUT / f"{stamp}-streams"
    for platform in dict.fromkeys(p for p, _, _ in plan):
        target = pick_device(binary, platform, device)
        if not target:
            print(f"no {platform} device available; skipped")
            continue
        device_servers = server_map(servers_path, binary, target["key"])
        for label in servers:
            server = device_servers[label]
            start = time.monotonic()
            if label == "agent-mobile":
                subprocess.run([str(binary), "--device", target["key"], "status"], capture_output=True)
            with tempfile.TemporaryDirectory() as cwd:
                run_claude(WARMUP_GOAL, label, server, cwd, keep=streams / f"{platform}-{label}-warmup")
            warmup[f"{label}:{platform}"] = round((time.monotonic() - start) * 1000)
            for _, task, seed in (x for x in plan if x[0] == platform):
                print(f"{platform} {label} {task['id']} seed {seed}", flush=True)
                rows.append(run_one(label, server, task, platform, seed, target["key"], streams, device_servers))
        teardown_all(device_servers, target["key"])
    result = {"meta": {"claude": version, "model": CLAUDE_MODEL, "stamp": time.strftime("%Y-%m-%d %H:%M"),
                       "warmup_ms": warmup}, "agent_tasks": rows}
    OUT.mkdir(exist_ok=True)
    path = OUT / (stamp + "-agent.json")
    path.write_text(json.dumps(result))
    from scripts.benchkit.report import render_agent
    render_agent(result, path.with_suffix(".html"))
    print(f"results: {path}")
    return result
