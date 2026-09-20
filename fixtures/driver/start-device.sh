#!/bin/sh
# Start the agent-mobile driver on a physical iPhone. The driver binds 0.0.0.0 on the phone so the
# Mac can reach it over Wi-Fi (e.g. http://Lahfirs-iPhone.local:8770). Bearer token required on every call.
# Usage: driver/start-device.sh <device-udid> [port]      (token: $AGENT_MOBILE_TOKEN, generated if unset)
cd "$(dirname "$0")" || exit 1
UDID="${1:?usage: start-device.sh <device-udid> [port]}"; PORT="${2:-8770}"
TOKEN="${AGENT_MOBILE_TOKEN:-$(openssl rand -hex 12)}"
umask 077
echo "$TOKEN" > /tmp/agent-mobile-device-token
chmod 600 /tmp/agent-mobile-device-token
nohup env TEST_RUNNER_AGENT_MOBILE_PORT="$PORT" TEST_RUNNER_AGENT_MOBILE_TOKEN="$TOKEN" TEST_RUNNER_AGENT_MOBILE_BIND=0.0.0.0 \
  xcodebuild test -project ToDo.xcodeproj -scheme ToDo -destination "platform=iOS,id=$UDID" \
  -only-testing:ToDoUITests/AgentMobileServer/testServe -skip-testing:ToDoTests \
  -parallel-testing-enabled NO -derivedDataPath ./dd-device -allowProvisioningUpdates \
  > /tmp/agent-mobile-device-driver.log 2>&1 &
echo "xcodebuild started (pid $!). log: /tmp/agent-mobile-device-driver.log  token: /tmp/agent-mobile-device-token"
