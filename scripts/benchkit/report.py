"""Run metadata and HTML report rendering."""
import json
import time

from scripts.benchkit.config import REPORT_TEMPLATE, ROOT
from scripts.benchkit.core import sh
from scripts.benchkit.select import select_device


def meta(b, runs, match):
    dev = select_device(json.loads(b.call("devices")[2] or b"{}").get("devices", []), match) or {}
    m = {"stamp": time.strftime("%Y-%m-%d %H:%M"), "runs": runs, "device": b.device, "os": dev.get("os"),
         "platform": b.platform,
         "sha": sh("git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"),
         "dirty": bool(sh("git", "-C", str(ROOT), "status", "--porcelain")),
         "cli": sh(b.bin, "--version")}
    if b.platform == "android":
        m["avd"], m["apk"] = b.avd, b.apk
        m["adb"] = sh(b.adb, "--version").splitlines()[0] if b.adb else None
    else:
        m["xcode"] = sh("xcodebuild", "-version").replace("\n", " ")
    return m


def render(run, base, path):
    data = f"const RUN = {json.dumps(run)};\nconst BASE = {json.dumps(base)};"
    html = REPORT_TEMPLATE.read_text().replace("/*DATA*/", data)
    path.write_text(html)
    print(f"report: {path}")


def render_agent(run, path):
    data = f"const RUN = {json.dumps(run)};\nconst BASE = null;"
    html = REPORT_TEMPLATE.read_text().replace("/*DATA*/", data)
    path.write_text(html)
    print(f"report: {path}")
