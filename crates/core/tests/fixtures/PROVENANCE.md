# Fixture provenance

Every envelope here was recorded from a live driver response via
`scripts/record-fixtures.sh` — none is fabricated or hand-written. All are
verbatim driver bytes except `*-screenshot.json`, whose `png_base64` is
deliberately truncated to 2048 chars on a 4-char base64 boundary and the
envelope re-serialized to compact JSON.

Re-record only through a live session, with placeholders — never a bearer
value on the command line:

```
AGENT_MOBILE_URL=<driver-url> AGENT_MOBILE_TOKEN_FILE=<0600-token-file> \
    scripts/record-fixtures.sh android   # or: ios
```

## iOS fixtures (unprefixed names)

`status`, `snapshot-springboard`, `error-*`, `screenshot`,
`snapshot-calendar`, `terminate` — recorded against the real iOS simulator
driver in earlier sessions; historical source context lives in git history
and Experiments 5–9.

## Android fixtures (`android-` prefix)

Recorded 2026-10-05, source commit `abf3877`, against:

- AVD `agent-mobile-api37` — Android 17 / API 37, Google APIs, arm64 emulator
- emulator binary 37.2.12 (build 16428233), `adb` 37.0.1-15733141
- driver package `com.lahfir.agentmobile.driver`, device listener port 8770
- foreground fixture app `com.google.android.deskclock`
- traffic carried through the U6 local bridge (loopback → owned ADB forward)

Notes:

- `android-screenshot` keeps a real PNG prefix: `png_base64` is truncated to
  2048 chars only on a valid 4-char base64 boundary; decode still starts
  with PNG magic.
- `android-error-ambiguous` is a genuinely staged `AMBIGUOUS_TARGET` (a real
  duplicate-identity node pair on the live tree), not a fabricated envelope.
- Dynamic values — snapshot ids, elapsed timings, refs, ports — remain real
  recorded values; the test suite's filters normalize them.
- No bearer token is ever serialized into a fixture; unauthorized fixtures
  use a static wrong-token header.
