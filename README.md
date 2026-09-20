# agent-mobile

Let an AI agent drive iOS (and later Android) apps through a snapshot -> act loop, on the
simulator and on a physical device, with the driver exposed on a plain local HTTP port so any
tunnel can forward it. Sibling of agent-browser (web) and agent-desktop (native desktop).

## Status (2026-09-19)

- Research complete: `docs/` holds 13 track files plus `docs/research/README.md` (the synthesis). Every
  claim there is tagged `[VERIFIED]` or `[INFERRED]`.
- Probe driver built: `fixtures/driver/ToDoUITests/AgentMobileServer.swift`, 267 lines. A never-ending
  XCUITest method hosts a tiny HTTP server (WebDriverAgent shape). XCUITest is the hidden
  mechanism; the agent only sees HTTP.
- Proven on the iOS 26 simulator (`docs/experiments/RESULTS.md`, Experiment 5): agent-driven
  launch -> snapshot -> tap by ref -> type by ref -> tap Done -> re-snapshot showed a new
  Calendar event. No scripted test; each step was a separate HTTP call chosen after reading the
  previous reply.
- Proven through a cloudflared quick tunnel (Experiment 6): same driver, public HTTPS URL,
  status 200 in 0.45 s, settled snapshot in 0.51 s, 401 without the token.
- Proven on a physical iPhone 14 Pro, iOS 27.0, over Wi-Fi and through a public cloudflared
  tunnel (Experiment 7): the same driver binary signed with a personal Apple Development
  identity answered status in 0.14 s on the LAN and 0.87 s through the tunnel; launched Calendar
  (2.0 s, 136 refs); tapped Add by ref (2.2 s, 252 refs). A type call hit STALE_REF once (the
  sheet had still been moving), the driver refused it as designed, a re-snapshot took 0.56 s.
  The run was stopped there by the user.
- Full loop proven on that same phone (Experiment 8): launch Calendar (2.04 s, 136 refs) ->
  tap Add (2.06 s, 252 refs) -> type the title (2.29 s, 253 refs) -> tap Done (1.84 s, 138 refs),
  and the returned day view carries `button "Agent Mobile Probe, from 7:00 PM to 8:00 PM"`.
  Four agent-chosen HTTP calls, 8.2 s of driver time, no STALE_REF. Screenshot:
  `docs/experiments/logs/21_device_calendar_event_created.png`.
- Open: the two-read tree hash declared settle while a late layout pass was still pending on
  the physical device (Experiment 7); a third read or a minimum settle window is the obvious
  next step. Experiment 8 did not reproduce it and did not find its trigger; it did document a
  different staleness path (a Title ref taken on the empty field dies once text goes in).
- Open: the Developer App certificate trust on the phone was not in effect an hour after
  Experiment 7 granted it, and had to be granted by hand again before Experiment 8
  (Settings > General > VPN & Device Management). Cause unknown.
- Not built (deliberately, this was a probe): the agent-facing CLI, an MCP wrapper, Android,
  the iOS in-app SDK, end-to-end encryption, swipe/scroll/long-press exercise, AMBIGUOUS_TARGET
  exercise, list dedup in the snapshot text.

## Layout

- `docs/PRD.md` — the product requirements: contract, phases P1–P4 with experiment exit criteria,
  engineering practices, risks, and the reliability gate.
- `fixtures/driver/` — the probe iOS driver: an Xcode project whose UI-test target hosts the HTTP
  server (`ToDoUITests/AgentMobileServer.swift`). The ToDo host app is only the scaffold a UI-test
  target needs; the driver never touches it. `am.sh` is a curl helper, `start-device.sh` starts the
  driver on a physical iPhone, `tunnel-cloudflared.sh` is the reference tunnel adapter.
- `docs/research/` and `docs/experiments/` — local reference material, gitignored and not published:
  the 13 research tracks with their synthesis, and the experiment record with verbatim output
  (Experiments 1–8) plus logs and screenshots. The PRD cites them by path.

## Run it

Simulator (boots the device if needed; the runner listens on `127.0.0.1:8770` of the Mac):

```
cd driver
TEST_RUNNER_AGENT_MOBILE_PORT=8770 TEST_RUNNER_AGENT_MOBILE_TOKEN=<token> \
xcodebuild test -project ToDo.xcodeproj -scheme ToDo \
  -destination 'platform=iOS Simulator,id=<sim-udid>' \
  -only-testing:ToDoUITests/AgentMobileServer/testServe -skip-testing:ToDoTests \
  -parallel-testing-enabled NO -derivedDataPath ./dd CODE_SIGNING_ALLOWED=NO
```

Note: `-parallel-testing-enabled NO` is required, otherwise Xcode drives a throwaway clone of
the simulator. `TEST_RUNNER_` is the prefix xcodebuild strips when passing environment into the
runner.

Physical iPhone (signs with the project's team, installs the host app and the runner, binds
`0.0.0.0` on the phone so the Mac reaches it over Wi-Fi):

```
fixtures/driver/start-device.sh <device-udid>        # token written to /tmp/agent-mobile-device-token
curl -X POST http://<phone-ip>:8770/status -H "Authorization: Bearer $(cat /tmp/agent-mobile-device-token)"
```

Note: on first install iOS refuses to launch the runner until the Developer App certificate is
trusted on the phone: Settings > General > VPN & Device Management > trust the certificate,
then run the script again. The phone shows "Automation Running" while the driver is up.

Tunnel (any forwarder works; the driver knows nothing about it):

```
fixtures/driver/tunnel-cloudflared.sh                       # simulator: forwards http://localhost:8770
fixtures/driver/tunnel-cloudflared.sh http://<phone-ip>:8770  # physical device
# alternatives: ngrok http 8770 | tailscale funnel 8770 | bore local 8770 --to bore.pub | ssh -R
export AGENT_MOBILE_URL=https://<public-host> AGENT_MOBILE_TOKEN=<token>
fixtures/driver/am.sh snapshot '{}'
```

## Protocol

- Every call is `POST /<command>` with a JSON body and `Authorization: Bearer <token>`. Missing
  or wrong token -> 401.
- Commands: `status`; `launch {bundle_id}`; `activate {bundle_id}`; `terminate`;
  `snapshot {app?}`; `tap {ref} | {x,y}`; `type {text, ref?}`;
  `swipe {direction: up|down|left|right, ref?}`; `home`; `screenshot` (PNG base64).
- Response envelope: `{version, ok, command, elapsed_ms, data}` or
  `{version, ok:false, command, elapsed_ms, error:{code, message}}`. Error codes: STALE_REF,
  AMBIGUOUS_TARGET, BAD_REQUEST, UNKNOWN_COMMAND, UNAUTHORIZED, DRIVER_ERROR.
- `data` for snapshot and for every action: `app, snapshot_id, ref_count, complete, settled,
  reads, text, tree`. Every action returns the fresh post-action tree, so an action costs no
  extra round trip.
- Node: `role, name, value, ref_id, states, available_actions, native_id {kind:"ax_identifier",
  value}, bounds {x,y,w,h}, children` — the agent-desktop shape (`docs/research/00-agent-desktop-contract.md`).
- Refs are per-snapshot and qualified: `@<snapshot_id>:eN`. Each action re-resolves its ref
  against the live tree by element type + identifier + label + frame (1 pt tolerance).
  Snapshot-id mismatch or no live match -> STALE_REF; more than one match -> AMBIGUOUS_TARGET.
  No self-healing, by design (`docs/research/07`, `docs/research/11`).
- Idle check: after every action the driver re-reads the tree until two consecutive tree hashes
  match, capped at 3 s; `settled` and `reads` report what happened.
- `Accept: text/plain` returns the compact listing instead of JSON, one line per named or
  interactive node: `@id:eN role "name" value="..." at=x,y size=WxH [states]`.

## Security (probe-grade)

- Bearer token on every route.
- HTTPS to Cloudflare and an encrypted cloudflared hop to the Mac.
- The Mac-to-phone hop is plain HTTP on the LAN.
- The phone binds all interfaces.
- Cloudflare sees plaintext (no end-to-end encryption).
- Whoever holds the URL and token has full UI control.
- No rate limit, expiry, allowlist, or audit log.

Use a fresh token per session and only on a trusted Wi-Fi. The trust model for a product is in
`docs/research/10`.
