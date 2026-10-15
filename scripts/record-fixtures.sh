#!/usr/bin/env bash
# Record golden fixtures from a LIVE driver. Never hand-write fixture bytes.
#
# Usage: scripts/record-fixtures.sh [ios|android]
#   AGENT_MOBILE_URL         default http://127.0.0.1:8770
#   AGENT_MOBILE_TOKEN_FILE  0600 file holding the session bearer (preferred)
#   AGENT_MOBILE_FIXTURE_APP ios: com.apple.mobilecal / android: com.google.android.deskclock
#
# The token is read from AGENT_MOBILE_TOKEN_FILE (preferred — a 0600 session
# file) or, as a legacy fallback, copied privately from AGENT_MOBILE_TOKEN
# into the temp dir; either way it only ever reaches a private curl header
# file — never argv, output, or the fixtures.
set -euo pipefail
umask 077

MODE="${1:-ios}"
case "$MODE" in
    ios|android) ;;
    *) echo "usage: scripts/record-fixtures.sh [ios|android]" >&2; exit 2 ;;
esac

cd "$(git rev-parse --show-toplevel)"
OUT=crates/core/tests/fixtures
URL="${AGENT_MOBILE_URL:-http://127.0.0.1:8770}"
APP="${AGENT_MOBILE_FIXTURE_APP:-}"
[ -z "$APP" ] && { [ "$MODE" = ios ] && APP="com.apple.mobilecal" || APP="com.google.android.deskclock"; }
PREFIX=""; [ "$MODE" = android ] && PREFIX="android-"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/am-fixtures.XXXXXX")
trap 'rm -r -- "$TMP"' EXIT
BODY="$TMP/body"
AUTH_HDR="$TMP/auth.h"; WRONG_HDR="$TMP/wrong.h"

# Preferred path; AGENT_MOBILE_TOKEN is a legacy fallback only.
TOKEN_FILE="${AGENT_MOBILE_TOKEN_FILE:-}"
if [ -z "$TOKEN_FILE" ] && [ -n "${AGENT_MOBILE_TOKEN:-}" ]; then
    TOKEN_FILE="$TMP/legacy-token"
    printf '%s' "$AGENT_MOBILE_TOKEN" > "$TOKEN_FILE"
fi
[ -n "$TOKEN_FILE" ] && [ -r "$TOKEN_FILE" ] || {
    echo "set AGENT_MOBILE_TOKEN_FILE to the session token file" >&2; exit 1;
}
# Bearer goes into a private header file — an -H argv entry would expose it
# in `ps`; the static wrong-token header records the unauthorized shape.
printf 'Authorization: Bearer %s\n' "$(cat "$TOKEN_FILE")" > "$AUTH_HDR"
printf 'Authorization: Bearer %s\n' "fixture-wrong-token" > "$WRONG_HDR"

STATUS=""
post() { # post <verb> <body> [header-file] [version] -> body via $BODY, code via $STATUS
    local verb="$1" body="${2:-"{}"}" hdr="${3:-$AUTH_HDR}" ver="${4:-1}"
    STATUS=$(curl -s -m 60 -o "$BODY" -w '%{http_code}' \
        -X POST "$URL/$verb" \
        -H @"$hdr" \
        -H "X-Agent-Mobile-Version: $ver" \
        -H "Content-Type: application/json" \
        -d "$body")
}

record() { # record <name> <verb> <body> [header-file] [version] — body only
    local name="$1"; shift
    post "$@"
    cp "$BODY" "$OUT/$name.json"
    printf '%-28s http=%s bytes=%s\n' "$name" "$STATUS" "$(wc -c < "$OUT/$name.json" | tr -d ' ')"
}

mkdir -p "$OUT"

echo "== status =="
record "${PREFIX}status" status

APP_BODY=$(python3 -c 'import json, sys; print(json.dumps({"bundle_id": sys.argv[1]}, separators=(",",":")))' "$APP")

if [ "$MODE" = android ]; then
    echo "== foreground fixture app =="
    post launch "$APP_BODY"
    [ "$STATUS" = "200" ] || { echo "launch $APP failed (http $STATUS)" >&2; exit 1; }
    record "android-snapshot" snapshot '{}'
else
    record snapshot-springboard snapshot '{}'
fi

echo "== errors =="
record "${PREFIX}error-bad-request" tap '{}'
record "${PREFIX}error-unauthorized" status '{}' "$WRONG_HDR"
record "${PREFIX}error-version-mismatch" status '{}' "$AUTH_HDR" '0'
record "${PREFIX}error-unknown-command" bogus

echo "== screenshot (base64 truncated on a 4-char boundary) =="
post screenshot '{}'
python3 - "$BODY" "$OUT/${PREFIX}screenshot.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
png = d.get('data', {}).get('png_base64', '')
d['data']['png_base64'] = png[:2048 - len(png) % 4] if png else png
with open(sys.argv[2], 'w') as f:
    json.dump(d, f, separators=(',', ':'))
    f.write('\n')
print('screenshot png_base64 truncated to', len(d['data']['png_base64']), 'chars')
PY

echo "== stale ref =="
post snapshot '{}'
REF=$(python3 - "$BODY" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
def walk(n):
    yield n
    for c in n.get('children', []):
        yield from walk(c)
print(next((n['ref_id'] for n in walk(d.get('data', {}).get('tree', {})) if n.get('ref_id')), ''))
PY
)
[ -n "$REF" ] || { echo "no ref in snapshot; cannot stage stale" >&2; exit 1; }
post snapshot '{}' > /dev/null
record "${PREFIX}error-stale" tap "{\"ref\":\"$REF\"}"

echo "== ambiguous target (real duplicate detection) =="
detect_dups() {
    python3 - "$BODY" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
groups = {}
def walk(n):
    key = (n.get('role',''), n.get('name',''), n.get('value',''),
           (n.get('native_id') or {}).get('value',''),
           tuple(round(v) for v in n.get('bounds',{}).values()))
    groups.setdefault(key, []).append(n.get('ref_id',''))
    for c in n.get('children', []): walk(c)
walk(d.get('data', {}).get('tree', {}))
for refs in groups.values():
    if len(refs) > 1:
        print(' '.join(refs))
PY
}
AMBIGUOUS=0
for _ in 1 2 3 4 5; do
    post snapshot '{}'
    CANDIDATES=$(detect_dups | awk '{print $1}')
    [ -z "$CANDIDATES" ] && break
    for ref in $CANDIDATES; do
        post tap "{\"ref\":\"$ref\"}"
        code=$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("error",{}).get("code",""))' < "$BODY" || true)
        if [ "$code" = "AMBIGUOUS_TARGET" ]; then
            cp "$BODY" "$OUT/${PREFIX}error-ambiguous.json"
            printf '%-28s http=%s bytes=%s\n' "${PREFIX}error-ambiguous" "$STATUS" "$(wc -c < "$BODY" | tr -d ' ')"
            AMBIGUOUS=1; break
        fi
        [ "$STATUS" = "200" ] && break   # a real tap landed; re-snapshot next round
    done
    [ "$AMBIGUOUS" = "1" ] && break
done
if [ "$AMBIGUOUS" != "1" ]; then
    if [ "$MODE" = android ]; then
        echo "FATAL: could not stage a real AMBIGUOUS_TARGET on Android" >&2
        exit 1
    fi
    echo "WARNING: could not stage AMBIGUOUS_TARGET; fixture not recorded" >&2
fi

echo "== app snapshot + terminate =="
if [ "$MODE" = ios ]; then
    record snapshot-calendar launch "$APP_BODY"
    record terminate terminate
else
    record android-terminate terminate
fi
post home '{}' > /dev/null   # leave the device on its launcher
echo "done -> $OUT"
