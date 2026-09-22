#!/usr/bin/env bash
# Record golden fixtures from a LIVE driver. Never hand-write fixture bytes.
# Requires a running driver (simulator or device):
#   AGENT_MOBILE_URL   default http://127.0.0.1:8770
#   AGENT_MOBILE_TOKEN required
# Usage: scripts/record-fixtures.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
OUT=crates/core/tests/fixtures
URL="${AGENT_MOBILE_URL:-http://127.0.0.1:8770}"
: "${AGENT_MOBILE_TOKEN:?set AGENT_MOBILE_TOKEN for the running driver}"
mkdir -p "$OUT"

# The bearer travels in a header read from a private temp file; an
# -H "Authorization: ..." argv entry would expose it to `ps` for every
# local user while the curl runs.
AUTH_HDR=$(mktemp -t am-auth)
trap 'rm -f "$AUTH_HDR" /tmp/am-fixture-body /tmp/am-fixture-status' EXIT
printf 'Authorization: Bearer %s\n' "$AGENT_MOBILE_TOKEN" > "$AUTH_HDR"

post() { # post <verb> <json> [token] [version] -> body on stdout, status via $STATUS
    local verb="$1" body="${2:-"{}"}" token="${3:-}" ver="${4:-1}"
    local hdr="$AUTH_HDR" own_hdr=""
    if [ -n "$token" ]; then
        own_hdr=$(mktemp -t am-auth-override)
        printf 'Authorization: Bearer %s\n' "$token" > "$own_hdr"
        hdr="$own_hdr"
    fi
    curl -s -m 60 -o /tmp/am-fixture-body -w '%{http_code}' \
        -X POST "$URL/$verb" \
        -H @"$hdr" \
        -H "X-Agent-Mobile-Version: $ver" \
        -H "Content-Type: application/json" \
        -d "$body" > /tmp/am-fixture-status
    [ -n "$own_hdr" ] && rm -f "$own_hdr"
    STATUS=$(cat /tmp/am-fixture-status)
    cat /tmp/am-fixture-body
}

record() { # record <name> <verb> <json> [token] [version]
    local name="$1"; shift
    post "$@" > /dev/null
    cp /tmp/am-fixture-body "$OUT/$name.json"
    printf '%-28s http=%s bytes=%s\n' "$name" "$STATUS" "$(wc -c < "$OUT/$name.json" | tr -d ' ')"
}

jget() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))' "$1"; }

echo "== simple shapes =="
record status status
record snapshot-springboard snapshot '{}'
record error-bad-request tap '{}'
record error-unauthorized status '{}' 'wrong-token'
record error-version-mismatch status '{}' "$AGENT_MOBILE_TOKEN" '0'
record error-unknown-command bogus

echo "== screenshot (base64 truncated to keep the fixture small) =="
post screenshot '{}' > /dev/null
python3 - <<'PY'
import json
d = json.load(open('/tmp/am-fixture-body'))
png = d.get('data', {}).get('png_base64', '')
d['data']['png_base64'] = png[:2048 - len(png) % 4] if png else png
json.dump(d, open('crates/core/tests/fixtures/screenshot.json', 'w'))
print('screenshot fixture: png_base64 truncated to', len(d['data']['png_base64']), 'chars')
PY

echo "== stale ref =="
post snapshot '{}' > /dev/null
REF=$(jget "d['data']['tree']['children'][0]['ref_id']" < /tmp/am-fixture-body)
post snapshot '{}' > /dev/null   # supersede: refs minted for the new snapshot id
record error-stale tap "{\"ref\":\"$REF\"}"

echo "== ambiguous target =="
detect_dups() {
    python3 - <<'PY'
import json
d = json.load(open('/tmp/am-fixture-body'))
tree = d.get('data', {}).get('tree', {})
groups = {}
def walk(n):
    key = (n.get('role',''), n.get('name',''), n.get('value',''),
           (n.get('native_id') or {}).get('value',''),
           tuple(round(v) for v in n.get('bounds',{}).values()))
    groups.setdefault(key, []).append(n.get('ref_id',''))
    for c in n.get('children', []): walk(c)
walk(tree)
for refs in groups.values():
    if len(refs) > 1:
        print(' '.join(refs))
PY
}
> "$OUT/error-ambiguous.json.attempted"
for round in 1 2 3 4 5; do
    post snapshot '{}' > /dev/null
    CANDIDATES=$(detect_dups | awk '{print $1}')
    [ -z "$CANDIDATES" ] && { echo "no duplicate nodes found in snapshot"; break; }
    found=0
    for ref in $CANDIDATES; do
        post tap "{\"ref\":\"$ref\"}" > /dev/null
        code=$(jget "d.get('error',{}).get('code','')" < /tmp/am-fixture-body || true)
        if [ "$code" = "AMBIGUOUS_TARGET" ]; then
            cp /tmp/am-fixture-body "$OUT/error-ambiguous.json"
            echo "ambiguous recorded via $ref"
            found=1; break
        fi
        if [ "$STATUS" = "200" ]; then break; fi  # a real tap landed; re-snapshot next round
    done
    [ "$found" = "1" ] && break
done
if [ ! -s "$OUT/error-ambiguous.json" ]; then
    echo "WARNING: could not stage AMBIGUOUS_TARGET; fixture not recorded" >&2
fi

echo "== calendar snapshot + terminate =="
record snapshot-calendar launch '{"bundle_id":"com.apple.mobilecal"}'
record terminate terminate
post home '{}' > /dev/null   # leave the sim on springboard
echo "done -> $OUT"
