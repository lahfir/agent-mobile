#!/bin/sh
# probe helper: am <command> ['{"json":"body"}']  -> text listing (or JSON with AM_JSON=1)
URL="${AGENT_MOBILE_URL:-http://localhost:8770}"
if [ -z "${AGENT_MOBILE_TOKEN:-}" ]; then
  echo "am.sh: AGENT_MOBILE_TOKEN is not set; refusing to send a default token" >&2
  exit 1
fi
TOKEN="$AGENT_MOBILE_TOKEN"
ACCEPT="text/plain"; [ -n "$AM_JSON" ] && ACCEPT="application/json"
curl -s -m 60 -X POST "$URL/$1" -H "Authorization: Bearer $TOKEN" -H "Accept: $ACCEPT" -H "Content-Type: application/json" -H "X-Agent-Mobile-Version: 1" -d "${2:-{\}}"
echo
