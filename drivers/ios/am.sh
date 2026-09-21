#!/bin/sh
# probe helper: am <command> ['{"json":"body"}']  -> text listing (or JSON with AM_JSON=1)
URL="${AGENT_MOBILE_URL:-http://localhost:8770}"
if [ -z "${AGENT_MOBILE_TOKEN:-}" ]; then
  echo "am.sh: AGENT_MOBILE_TOKEN is not set; refusing to send a default token" >&2
  exit 1
fi
ACCEPT="text/plain"; [ -n "$AM_JSON" ] && ACCEPT="application/json"
HDR=$(mktemp -t am-auth) || exit 1
trap 'rm -f "$HDR"' EXIT INT TERM
# The bearer travels in a header read from a private file; -H "Bearer ..." in
# argv would expose it to `ps` for every local user.
printf 'Authorization: Bearer %s\n' "$AGENT_MOBILE_TOKEN" > "$HDR"
curl -s -m 60 -X POST "$URL/$1" -H @"$HDR" -H "Accept: $ACCEPT" -H "Content-Type: application/json" -H "X-Agent-Mobile-Version: 1" -d "${2:-{\}}"
echo
