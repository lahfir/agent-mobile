#!/bin/sh
# probe helper: am <command> ['{"json":"body"}']  -> text listing (or JSON with AM_JSON=1)
URL="${AGENT_MOBILE_URL:-http://localhost:8770}"
TOKEN="${AGENT_MOBILE_TOKEN:-probe-token}"
ACCEPT="text/plain"; [ -n "$AM_JSON" ] && ACCEPT="application/json"
curl -s -m 60 -X POST "$URL/$1" -H "Authorization: Bearer $TOKEN" -H "Accept: $ACCEPT" -H "Content-Type: application/json" -d "${2:-{\}}"
echo
