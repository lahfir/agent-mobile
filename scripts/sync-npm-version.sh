#!/usr/bin/env bash
# Sync the npm package with the crate and stage the prebuilt artifacts the
# tarball ships (KTD14):
#   npm/bin/agent-mobile   <- cargo build --release
#   npm/runner/            <- xcodebuild build-for-testing products
# Run from the repo root before `npm pack`; the artifacts are gitignored
# build output, not committed source.
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
cp target/release/agent-mobile npm/bin/agent-mobile
chmod 755 npm/bin/agent-mobile
echo "staged bin/agent-mobile ($(target/release/agent-mobile --version))"

# 3. Runner: build-for-testing products plus the xctestrun manifest. The
#    manifest's __TESTROOT__ paths resolve to npm/runner/ when Debug-* sits
#    beside it.
SIM_UDID=$(xcrun simctl list devices available --json \
  | python3 -c 'import json,sys
devs = json.load(sys.stdin)["devices"]
sims = [d for rt, ds in devs.items() if "iOS" in rt for d in ds if "iPhone" in d["name"]]
print(sims[0]["udid"] if sims else "")')
[ -n "$SIM_UDID" ] || { echo "no iPhone simulator found; create one first" >&2; exit 1; }

(cd fixtures/driver && xcodebuild build-for-testing \
  -project ToDo.xcodeproj -scheme ToDo \
  -destination "platform=iOS Simulator,id=$SIM_UDID" \
  -derivedDataPath ./dd CODE_SIGNING_ALLOWED=NO >/dev/null)

rm -rf npm/runner
mkdir -p npm/runner
cp fixtures/driver/dd/Build/Products/*.xctestrun npm/runner/
cp -R fixtures/driver/dd/Build/Products/Debug-iphonesimulator npm/runner/
echo "staged npm/runner ($(ls npm/runner/*.xctestrun | wc -l | tr -d ' ') xctestrun, $(du -sh npm/runner | cut -f1))"
