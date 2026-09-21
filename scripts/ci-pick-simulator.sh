#!/usr/bin/env bash
# Print SIM_UDID, SIM_NAME and SIM_RUNTIME for the first available iPhone
# simulator, in the key=value form GitHub Actions appends to $GITHUB_ENV.
set -euo pipefail

xcrun simctl list devices available --json | python3 -c '
import json, sys

devices = json.load(sys.stdin)["devices"]
for runtime, entries in devices.items():
    for device in entries:
        if device.get("isAvailable") and "iPhone" in device["name"]:
            print("SIM_UDID=" + device["udid"])
            print("SIM_NAME=" + device["name"])
            print("SIM_RUNTIME=" + runtime.rsplit(".", 1)[-1])
            sys.exit(0)
sys.exit("no available iPhone simulator")
'
