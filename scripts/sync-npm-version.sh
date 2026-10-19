#!/usr/bin/env bash
# Sync the npm package with the crate and stage the prebuilt artifacts the
# tarball ships (KTD14):
#   npm/bin/agent-mobile   <- cargo build --release
#   npm/runner/            <- xcodebuild build-for-testing products
#   npm/android/           <- gradle assembleDebug driver APK
# Run from the repo root before `npm pack`; the artifacts are gitignored
# build output, not committed source. The package's bin is the native binary
# itself, so the tarball needs no install script; this script is where the
# binary and runner are checked.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# 1. Version: crate is the source of truth; the protocol version stays put.
CRATE_VER=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$CRATE_VER" ] || { echo "cannot read workspace version" >&2; exit 1; }
sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$CRATE_VER\"/" npm/package.json
echo "npm package version -> $CRATE_VER"

# 2. Binary: release build staged at npm/bin/agent-mobile.
cargo build --release --locked
mkdir -p npm/bin
# A new inode: overwriting a signed binary in place makes macOS kill it (signal 9).
rm -f npm/bin/agent-mobile
cp target/release/agent-mobile npm/bin/agent-mobile
chmod 755 npm/bin/agent-mobile
npm/bin/agent-mobile --version >/dev/null || { echo "staged binary does not run" >&2; exit 1; }
echo "staged bin/agent-mobile ($(npm/bin/agent-mobile --version))"

# 3. Runner: build-for-testing products plus the xctestrun manifest. The
#    manifest's __TESTROOT__ paths resolve to npm/runner/ when Debug-* sits
#    beside it.
SIM_UDID=$(xcrun simctl list devices available --json \
  | python3 -c 'import json,sys
devs = json.load(sys.stdin)["devices"]
sims = [d for rt, ds in devs.items() if "iOS" in rt for d in ds if "iPhone" in d["name"]]
print(sims[0]["udid"] if sims else "")')
[ -n "$SIM_UDID" ] || { echo "no iPhone simulator found; create one first" >&2; exit 1; }

(cd drivers/ios && xcodebuild build-for-testing \
  -project AgentMobileDriver.xcodeproj -scheme AgentMobileDriver \
  -destination "platform=iOS Simulator,id=$SIM_UDID" \
  -derivedDataPath ./dd CODE_SIGNING_ALLOWED=NO >/dev/null)

rm -rf npm/runner
mkdir -p npm/runner
cp drivers/ios/dd/Build/Products/*.xctestrun npm/runner/
cp -R drivers/ios/dd/Build/Products/Debug-iphonesimulator npm/runner/
ls npm/runner/*.xctestrun >/dev/null 2>&1 || { echo "no xctestrun staged in npm/runner" >&2; exit 1; }
echo "staged npm/runner ($(ls npm/runner/*.xctestrun | wc -l | tr -d ' ') xctestrun, $(du -sh npm/runner | cut -f1))"

# 4. Android driver APK: the CLI uses bin/../android/ before any Gradle build.
drivers/android/gradlew -p drivers/android :app:assembleDebug --no-daemon -q
rm -rf npm/android
mkdir -p npm/android
cp drivers/android/app/build/outputs/apk/debug/app-debug.apk npm/android/agent-mobile-driver.apk
echo "staged npm/android ($(du -sh npm/android | cut -f1))"

# 5. A global link left from an older layout points at a file that no longer exists.
if link=$(command -v agent-mobile) && [ ! -e "$(readlink -f "$link")" ]; then
  echo "warning: $link points at a missing file; run \`npm link\` in npm/ to relink it" >&2
fi
