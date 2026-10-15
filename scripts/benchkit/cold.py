"""Cold-boot flows: simulator reboot (iOS) and emulator reboot (Android)."""
import os
import subprocess
import time
from pathlib import Path

from scripts.benchkit.config import APPS, OUT, ROOT
from scripts.benchkit.core import sh
from scripts.benchkit.driver import _sweep_serves, stop_driver


def cold(b, target):
    if b.platform == "android":
        return cold_android(b)
    return cold_ios(b, target)


def cold_ios(b, udid):
    stop_driver(b.device)
    sh("xcrun", "simctl", "shutdown", udid)
    start = time.perf_counter()
    sh("xcrun", "simctl", "boot", udid)
    sh("xcrun", "simctl", "bootstatus", udid, "-b")
    boot = (time.perf_counter() - start) * 1000
    driver = b.call("status")[0]
    snap = b.call("snapshot")[0]
    b.cold.append({"boot": round(boot), "driver": round(driver), "snapshot": round(snap)})


def android_sdk():
    """(adb, emulator) binaries, resolved like the driver resolves them."""
    roots = [os.environ.get("ANDROID_HOME"), os.environ.get("ANDROID_SDK_ROOT"),
             str(Path.home() / "Library" / "Android" / "sdk")]
    for root in roots:
        if not root:
            continue
        adb, emulator = Path(root) / "platform-tools" / "adb", Path(root) / "emulator" / "emulator"
        if adb.is_file() and emulator.is_file():
            return str(adb), str(emulator)
    raise SystemExit("no Android SDK found; set ANDROID_HOME (see scripts/setup-android-sdk.sh --check)")


def driver_apk():
    """Build the driver APK once and pin it through AGENT_MOBILE_ANDROID_APK.

    Without the pin every lazy `serve` start runs a cold `gradlew --no-daemon`
    beside the booting emulator: minutes of Gradle inside the cold "driver"
    number, or a 180 s verb timeout on a loaded host."""
    if os.environ.get("AGENT_MOBILE_ANDROID_APK"):
        return os.environ["AGENT_MOBILE_ANDROID_APK"]
    android = ROOT / "drivers" / "android"
    print("building driver apk", flush=True)
    p = subprocess.run([str(android / "gradlew"), "-p", str(android), ":app:assembleDebug", "--no-daemon", "-q"])
    apk = android / "app" / "build" / "outputs" / "apk" / "debug" / "app-debug.apk"
    if p.returncode or not apk.is_file():
        raise SystemExit("driver apk build failed; run drivers/android/gradlew -p drivers/android :app:assembleDebug")
    os.environ["AGENT_MOBILE_ANDROID_APK"] = str(apk)
    return str(apk)


def android_serial(adb, avd):
    """Serial of the booted emulator claiming `avd`, or None. Only an `emu avd
    name` match counts — never position, never the recorded pid alone."""
    devices = subprocess.run([adb, "devices"], capture_output=True, text=True).stdout
    for line in devices.splitlines():
        serial = line.split()[0] if line.split() else ""
        if not serial.startswith("emulator-") or not line.rstrip().endswith("device"):
            continue
        name = subprocess.run([adb, "-s", serial, "emu", "avd", "name"],
                              capture_output=True, text=True).stdout
        if name.split() and name.split()[0] == avd:
            return serial
    return None


def cold_android(b):
    avd = b.avd
    if avd is None and b.selected_kind == "emulator":
        serials = [l.split()[0] for l in subprocess.run(
            [b.adb, "devices"], capture_output=True, text=True).stdout.splitlines()
            if l.split() and l.split()[0].startswith("emulator-")]
        names = {s: subprocess.run([b.adb, "-s", s, "emu", "avd", "name"],
                                   capture_output=True, text=True).stdout.split()[:1] for s in serials}
        matches = [s for s, n in names.items() if n]
        avd = b.avd = names[matches[0]][0] if len(matches) == 1 else None
    if avd is None:
        raise SystemExit("cold boot needs an emulator AVD (physical device?); rerun with --cold 0")
    stop_driver(b.device)
    _sweep_serves(b.device)
    serial = android_serial(b.adb, avd)
    if serial:
        subprocess.run([b.adb, "-s", serial, "emu", "kill"], capture_output=True)
        deadline = time.time() + 60
        while android_serial(b.adb, avd) and time.time() < deadline:
            time.sleep(2)
        if android_serial(b.adb, avd):
            raise SystemExit(f"emulator {avd} survived `emu kill`; aborting before it is timed as a cold boot")
    OUT.mkdir(exist_ok=True)
    log = (OUT / "emulator.log").open("w")
    start = time.perf_counter()
    subprocess.Popen([b.emulator, "-avd", avd, "-no-window", "-no-audio", "-no-boot-anim",
                      "-no-snapshot-load", "-no-snapshot-save", "-gpu", "swiftshader_indirect"],
                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    budget = int(os.environ.get("AGENT_MOBILE_BOOT_BUDGET_SECS", "600"))
    serial, booted = None, False
    while time.perf_counter() - start < budget:
        serial = android_serial(b.adb, avd)
        if serial:
            done = subprocess.run([b.adb, "-s", serial, "shell", "getprop", "sys.boot_completed"],
                                  capture_output=True, text=True).stdout.strip()
            if done == "1":
                booted = True
                break
        time.sleep(5)
    if not booted:
        raise SystemExit(f"emulator {avd} failed to boot within {budget}s; see {log.name}")
    for pkg in APPS["android"].values():
        subprocess.run([b.adb, "-s", serial, "shell", "appops", "set", pkg, "POST_NOTIFICATION", "ignore"],
                       capture_output=True)
    boot = (time.perf_counter() - start) * 1000
    driver = b.call("status")[0]
    snap = b.call("snapshot")[0]
    b.cold.append({"boot": round(boot), "driver": round(driver), "snapshot": round(snap)})
