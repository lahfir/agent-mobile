#!/usr/bin/env python3
"""Install a packed agent-mobile tarball with each package manager into its
own temp prefix, check every entry, time it, and print a JSON report with
gates.

    scripts/bench_install.py <tarball> [--mode default|allow] [--runs 7]
        [--pms npm,npm-local,pnpm,bun,yarn] [--out report.json]

Every install comes from the tarball path, never from the registry. A guard
`npm` answers `npm prefix -g` with a sentinel dir, and the real global links
are recorded first and restored if any install changes them. Installs and
correctness checks run as a fresh user (an empty HOME); timings that need
the live driver use the real HOME. Set
AM_BENCH_PNPM to a pnpm executable when the `pnpm` on PATH cannot start.
"""

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PKG = "agent-mobile"
WARNING = r"Ignored build scripts|approve-builds|Blocked \d+ postinstall|trustedDependencies"
COMMANDS = [["--version"], ["--help"], ["skills"], ["devices", "--platform", "ios"], ["status"]]
INIT = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "bench", "version": "0"}}}
MARKER_JS = """
const a = process.argv[1] || "";
if (a.endsWith("install.js") && a.includes("agent-mobile"))
  require("fs").appendFileSync(process.env.AM_BENCH_MARKER, a + "\\n");
"""
GUARD_NPM = """#!/bin/sh
if [ -z "${npm_config_prefix:-}" ] && [ "$1" = prefix ] && [ "$2" = -g ]; then
  echo "$AM_BENCH_SENTINEL"; exit 0
fi
exec "$AM_BENCH_REAL_NPM" "$@"
"""


def sh(cmd, env=None, cwd=None, timeout=300):
    try:
        r = subprocess.run(cmd, env=env, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except FileNotFoundError as e:
        return 127, str(e)
    return r.returncode, r.stdout + r.stderr


def real_links():
    """The real global `agent-mobile` links this run must never change."""
    env = {k: v for k, v in os.environ.items() if k != "npm_config_prefix"}
    prefix = sh(["npm", "prefix", "-g"], env=env)[1].strip()
    code, out = sh(["yarn", "global", "bin"], env=env)
    yarn_bin = out.strip().splitlines() if code == 0 else []
    paths = [Path(prefix, "bin", PKG), Path(prefix, "lib", "node_modules", PKG),
             Path.home() / ".bun" / "bin" / PKG]
    if os.environ.get("PNPM_HOME"):
        paths.append(Path(os.environ["PNPM_HOME"], PKG))
    if yarn_bin:
        paths.append(Path(yarn_bin[-1], PKG))
    return {str(p): os.readlink(p) if p.is_symlink() else ("file" if p.exists() else None) for p in paths}


def restore(recorded):
    """Put back any real link that changed; return the paths restored."""
    changed = []
    for path, target in recorded.items():
        p = Path(path)
        now = os.readlink(p) if p.is_symlink() else ("file" if p.exists() else None)
        if now == target:
            continue
        changed.append(path)
        if target in (None, "file"):
            print(f"WARNING: {path} changed and cannot be restored automatically", file=sys.stderr)
            continue
        if p.is_symlink() or p.exists():
            p.unlink()
        p.symlink_to(target)
    return changed


class Bench:
    def __init__(self, tarball, mode, runs):
        self.tarball = str(Path(tarball).resolve())
        self.mode = mode
        self.runs = runs
        self.root = Path(tempfile.mkdtemp(prefix="am-bench-", dir="/tmp"))
        self.sentinel = self.root / "sentinel"
        (self.sentinel / "bin").mkdir(parents=True)
        (self.sentinel / "bin" / PKG).symlink_to("/dev/null")
        guard = self.root / "guard"
        guard.mkdir()
        (guard / "npm").write_text(GUARD_NPM)
        (guard / "npm").chmod(0o755)
        (self.root / "marker.js").write_text(MARKER_JS)
        if os.environ.get("AM_BENCH_PNPM"):
            (guard / "pnpm").symlink_to(os.environ["AM_BENCH_PNPM"])
        self.logs = {}
        self.work = self.root / "work"
        self.work.mkdir()
        node_bin = str(Path(shutil.which("node")).parent)
        self.base_env = {
            **os.environ,
            "PATH": f"{guard}:{os.environ['PATH']}",
            "AM_BENCH_SENTINEL": str(self.sentinel),
            "AM_BENCH_REAL_NPM": str(Path(node_bin, "npm")),
            "NODE_OPTIONS": f"--require {self.root / 'marker.js'}",
            "SFW_SHIM_DISABLE": "1",
        }
        self.base_env.pop("npm_config_prefix", None)

    def install(self, pm):
        """Install with `pm`; return (entry, env, scripts, log)."""
        d = self.root / pm
        (d / "home").mkdir(parents=True)
        env = dict(self.base_env, AM_BENCH_MARKER=str(d / "marker"), HOME=str(d / "home"))
        allow = self.mode == "allow"
        if pm == "npm":
            env["npm_config_prefix"] = str(d / "prefix")
            cmd = ["npm", "install", "-g", "--foreground-scripts", self.tarball]
            entry = d / "prefix" / "bin" / PKG
        elif pm == "npm-local":
            (d / "package.json").write_text('{"name": "bench-local", "private": true}\n')
            cmd = ["npm", "install", "--foreground-scripts", self.tarball]
            entry = d / "node_modules" / ".bin" / PKG
        elif pm == "pnpm":
            env["PNPM_HOME"] = str(d / "pnpm")
            env["PATH"] = f"{d / 'pnpm' / 'bin'}:{d / 'pnpm'}:{env['PATH']}"
            cmd = ["pnpm", "add", "-g", f"--store-dir={d / 'store'}", self.tarball]
            if allow:
                cmd.insert(3, f"--allow-build={PKG}")
            entry = d / "pnpm" / "bin" / PKG
        elif pm == "bun":
            env["BUN_INSTALL"] = str(d / "bun")
            env["BUN_INSTALL_CACHE_DIR"] = str(d / "cache")
            cmd = ["bun", "add", "-g", self.tarball] + (["--trust"] if allow else [])
            entry = d / "bun" / "bin" / PKG
        elif pm == "yarn":
            cmd = ["yarn", "global", "add", self.tarball, f"--prefix={d / 'prefix'}",
                   f"--global-folder={d / 'global'}", f"--cache-folder={d / 'cache'}"]
            entry = d / "prefix" / "bin" / PKG
        else:
            raise SystemExit(f"unknown package manager {pm}")
        code, log = sh(cmd, env=env, cwd=d if pm == "npm-local" else self.work)
        self.logs[pm] = log
        if pm == "pnpm" and not entry.exists():
            entry = d / "pnpm" / PKG
        ran = (d / "marker").exists()
        blocked = any(m in log for m in ("Ignored build scripts", "Blocked", "blocked", "ignored"))
        if not self.postinstall:
            scripts = "none"
        else:
            scripts = "ran" if ran else ("blocked" if blocked else "skipped-silently")
        if code != 0:
            scripts = f"install-failed({code})"
        return entry, env, scripts, log[-2000:]

    def timed(self, argv, env, cwd=None):
        def once():
            t = time.perf_counter()
            subprocess.run(argv, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=120)
            return (time.perf_counter() - t) * 1000
        once()
        runs = [once() for _ in range(self.runs)]
        return {"min_ms": round(min(runs), 1), "median_ms": round(statistics.median(runs), 1)}

    def mcp(self, argv, env):
        """Spawn-to-reply times for initialize, tools/list, and a status call."""
        def once(tree=False):
            t = time.perf_counter()
            p = subprocess.Popen(argv + ["mcp"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL, text=True)
            marks = {}
            for name, msg in (("initialize", INIT),
                              ("tools_list", {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                              ("status_call", {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                                               "params": {"name": "status", "arguments": {}}})):
                p.stdin.write(json.dumps(msg) + "\n")
                p.stdin.flush()
                p.stdout.readline()
                marks[name] = (time.perf_counter() - t) * 1000
                if name == "initialize":
                    p.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            info = process_tree(p.pid) if tree else None
            p.stdin.close()
            p.wait(timeout=30)
            return marks, info
        once()
        samples = [once()[0] for _ in range(self.runs)]
        _, tree = once(tree=True)
        out = {k: {"min_ms": round(min(s[k] for s in samples), 1),
                   "median_ms": round(statistics.median(s[k] for s in samples), 1)} for k in samples[0]}
        out["tree"] = tree
        out["rss_kb"] = sum(row["rss_kb"] for row in tree)
        return out


def process_tree(root):
    rows = {}
    for line in sh(["ps", "-A", "-o", "pid=,ppid=,rss=,comm="])[1].splitlines():
        pid, ppid, rss, comm = line.split(None, 3)
        rows[int(pid)] = (int(ppid), int(rss), comm)
    keep, frontier = [], [root]
    while frontier:
        pid = frontier.pop()
        if pid in rows:
            keep.append({"pid": pid, "rss_kb": rows[pid][1], "comm": rows[pid][2]})
            frontier += [c for c, r in rows.items() if r[0] == pid]
    return keep


def chain(entry):
    p = Path(entry)
    info = {"entry": str(p), "readlink": os.readlink(p) if p.is_symlink() else None,
            "realpath": os.path.realpath(p), "file": sh(["file", "-bL", str(p)])[1].strip()}
    if "text" in info["file"]:
        info["head"] = p.read_text(errors="replace").splitlines()[:6]
    return info


def native_exe(entry, env):
    """The real path of the native binary an entry runs, from the live process."""
    p = subprocess.Popen([str(entry), "mcp"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True)
    p.stdin.write(json.dumps(INIT) + "\n")
    p.stdin.flush()
    p.stdout.readline()
    found = None
    for row in process_tree(p.pid):
        out = sh(["lsof", "-p", str(row["pid"]), "-a", "-d", "txt", "-Fn"])[1]
        paths = [line[1:] for line in out.splitlines() if line.startswith("n")]
        if paths and paths[0].endswith("/" + PKG):
            found = paths[0]
    p.stdin.close()
    p.wait(timeout=30)
    return found


def runner_beside(native):
    """The bundled runner sits at `<package>/runner`, beside `<package>/bin`."""
    return bool(native) and any((Path(native).parent.parent / "runner").glob("*.xctestrun"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("tarball")
    ap.add_argument("--mode", choices=["default", "allow"], default="default")
    ap.add_argument("--runs", type=int, default=7)
    ap.add_argument("--pms", default="npm,npm-local,pnpm,bun,yarn")
    ap.add_argument("--native", help="native binary to time as the floor")
    ap.add_argument("--out")
    args = ap.parse_args()
    recorded = real_links()
    bench = Bench(args.tarball, args.mode, args.runs)
    manifest = json.loads(sh(["tar", "-xzOf", bench.tarball, "package/package.json"])[1])
    version = manifest["version"]
    bench.postinstall = "postinstall" in manifest.get("scripts", {})
    report = {"tarball": bench.tarball, "mode": args.mode, "runs": args.runs, "entries": {}, "floors": {}}
    env = bench.base_env
    report["floors"]["sh -c exit"] = bench.timed(["sh", "-c", "exit"], env)
    report["floors"]["node -e 0"] = bench.timed(["node", "-e", "0"], env)
    if args.native:
        report["floors"]["native --version"] = bench.timed([args.native, "--version"], env)
    if args.native:
        live = dict(env, HOME=os.environ["HOME"])
        report["entries"]["native"] = {
            "scripts": "n/a", "works": True,
            "times": {" ".join(c): bench.timed([args.native, *c], live) for c in COMMANDS},
            "mcp": bench.mcp([args.native], live)}
    restored = []
    for pm in args.pms.split(","):
        entry, pm_env, scripts, log = bench.install(pm)
        restored += restore(recorded)
        row = {"scripts": scripts}
        if not entry.exists():
            row.update(works=False, log=log)
            report["entries"][pm] = row
            continue
        code, out = sh([str(entry), "--version"], env=pm_env)
        row["version_ok"] = code == 0 and out.strip() == f"{PKG} {version}"
        code, out = sh([str(entry), "skills"], env=pm_env)
        row["skills_ok"] = code == 0 and out.startswith("---\nname: agent-mobile")
        row["native"] = native_exe(entry, pm_env)
        row["runner_ok"] = runner_beside(row["native"])
        row["works"] = row["version_ok"] and row["skills_ok"] and row["runner_ok"]
        row["chain"] = chain(entry)
        live = dict(pm_env, HOME=os.environ["HOME"])
        row["times"] = {" ".join(c): bench.timed([str(entry), *c], live) for c in COMMANDS}
        row["mcp"] = bench.mcp([str(entry)], live)
        if pm == "npm-local":
            row["npx_version"] = bench.timed(["npx", "--no-install", PKG, "--version"], live,
                                             cwd=entry.parents[2])
        report["entries"][pm] = row
    report["gates"] = {
        "all_entries_work": all(e.get("works") for e in report["entries"].values()),
        "sentinel_unchanged": os.readlink(bench.sentinel / "bin" / PKG) == "/dev/null",
        "real_links_unchanged": not restored,
        "no_build_script_warning": not any(re.search(WARNING, log) for log in bench.logs.values()),
    }
    if restored:
        report["restored"] = restored
    text = json.dumps(report, indent=2)
    if args.out:
        Path(args.out).write_text(text)
    print(text)
    shutil.rmtree(bench.root, ignore_errors=True)
    return 0 if all(report["gates"].values()) else 1


if __name__ == "__main__":
    sys.exit(main())
