#!/bin/sh
# Start the agent-mobile driver on a physical iPhone. The driver binds 0.0.0.0 on the phone so the
# Mac can reach it over Wi-Fi (e.g. http://Lahfirs-iPhone.local:8770). Bearer token required on every call.
# Usage: drivers/ios/start-device.sh <device-udid> [port]      (token: $AGENT_MOBILE_TOKEN, generated if unset)
set -e
cd "$(dirname "$0")" || exit 1
UDID="${1:?usage: start-device.sh <device-udid> [port]}"; PORT="${2:-8770}"
TOKEN="${AGENT_MOBILE_TOKEN:-$(openssl rand -hex 12)}"
umask 077
mkdir -p "$HOME/.agent-mobile/tokens"
TOKEN_FILE="$HOME/.agent-mobile/tokens/device-manual"
printf '%s\n' "$TOKEN" > "$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"
nohup env TEST_RUNNER_AGENT_MOBILE_PORT="$PORT" TEST_RUNNER_AGENT_MOBILE_TOKEN="$TOKEN" TEST_RUNNER_AGENT_MOBILE_BIND=0.0.0.0 \
  xcodebuild test -project AgentMobileDriver.xcodeproj -scheme AgentMobileDriver -destination "platform=iOS,id=$UDID" \
  -only-testing:AgentMobileDriver/AgentMobileServer/testServe \
  -parallel-testing-enabled NO -derivedDataPath ./dd-device -allowProvisioningUpdates \
  > "$HOME/.agent-mobile/driver-device.log" 2>&1 &
echo "xcodebuild started (pid $!). log: $HOME/.agent-mobile/driver-device.log  token: $TOKEN_FILE"
