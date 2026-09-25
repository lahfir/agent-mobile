# agent-mobile

Let an AI agent drive iOS (and later Android) apps through a snapshot -> act loop, on the
simulator and on a physical device, with the driver exposed on a plain local HTTP port so any
tunnel can forward it. Sibling of agent-browser (web) and agent-desktop (native desktop).

## Status (2026-09-20)

- P1 shipped: the Rust CLI (`agent-mobile`) wraps the proven XCUITest driver. Any verb lazy-starts
  a driver when none runs; `serve` runs one in the foreground. The npm package bundles a prebuilt
  simulator runner, so the simulator path builds nothing on the user's Mac.
- Probe evidence stands (`docs/experiments/RESULTS.md`, Experiments 1–9): agent-driven Calendar
  events on the iOS 26 simulator and on a physical iPhone 14 Pro over Wi-Fi, settled snapshots,
  STALE_REF refusing a mistap, one boot from a clean state.
- Not built (later phases): Android (P2), the reliability benchmark (P3), the MCP wrapper (P4),
  the iOS in-app SDK, end-to-end encryption on the LAN hop.

## Install

macOS only, Node >= 18. The package ships a prebuilt CLI binary and a prebuilt simulator runner —
installation compiles nothing and downloads nothing.

```
npm i -g agent-mobile
agent-mobile --version
```

From a checkout instead (pins in `rust-toolchain.toml` and `Cargo.toml` apply):

```
cargo build --release --locked
```

## Quickstart

Any verb lazy-starts the driver when no session exists — boots the default simulator, builds the
runner on first use, saves the session under `~/.agent-mobile/`, then runs the verb. The first
call can take a minute; progress prints to stderr.

```
agent-mobile snapshot                  # lazy-boots, prints the accessibility tree
agent-mobile launch com.apple.mobilecal
agent-mobile tap @1abc:e7              # act on a ref from the last snapshot
agent-mobile type "Dentist tomorrow"   # type into the focused field
agent-mobile screenshot shot.png       # PNG file; omit the path for base64 on stdout
agent-mobile skills                    # the one-page agent guide
```

To run the driver explicitly (one per device, foreground, Ctrl-C cleans up):

```
agent-mobile devices                   # reachable simulators and paired devices
agent-mobile serve "iPhone 17 Pro Max" # prints the session token once when ready
```

## Commands

| Command | What it does |
|---|---|
| `devices` | list reachable simulators and paired devices |
| `serve <device>` | start a driver in the foreground; prints the session token once |
| `status` | active app, device, os, current snapshot id |
| `snapshot` | mint refs and print the accessibility tree |
| `tap <ref>` or `tap <x> <y>` | tap an element ref, or a point in the app frame |
| `type [<ref>] <text...>` | append text: a leading ref writes that field's value directly (no keyboard), otherwise keystrokes go to the focused field; use `--` before hyphen-leading text |
| `swipe <up\|down\|left\|right> [<ref>]` | swipe the app, or one element |
| `home` | press Home; returns the springboard tree |
| `launch <bundle_id>` | cold-start an app; kills saved state |
| `screenshot [path]` | PNG to a file, or base64 to stdout |
| `stop` | terminate the active app; the driver stays up |
| `skills` | print the one-page agent guide |

Global flags: `--json` (raw JSON envelope instead of text), `--app <bundle>` (retarget `snapshot`
at a bundle id), `--max-depth <n>` (trim the tree client-side; the header then shows
`complete=false`), `--device <name>` (pick a device; remembered for later calls).

## Environment

| Variable | Effect |
|---|---|
| `AGENT_MOBILE_URL` | driver URL override for the call (e.g. a tunnel or a phone on the LAN) |
| `AGENT_MOBILE_TOKEN` | bearer token paired with the URL override |
| `AGENT_MOBILE_DRIVER_DIR` | driver source override: an Xcode project dir or a `runner/` dir holding `*.xctestrun` |
| `TEST_RUNNER_AGENT_MOBILE_PORT` / `_TOKEN` / `_BIND` | driver-side env, set through xcodebuild's `TEST_RUNNER_` prefix (see `drivers/ios/am.sh`) |

## Sessions and state

Everything lives under `~/.agent-mobile/`:

- `state.json` — device entries: driver URL, serve pid, runner pid, token filename, and the
  remembered default device. Never holds a token value.
- `tokens/` — one `0600` file per session; the token itself.
- `driver-<device>.log` — the runner's log, written `0600` because the runner echoes its env.
- `serve.lock`, `boot.lock` — one serve at a time; one lazy boot at a time.

A second `serve` on a live device reports the URL, the pid, and the remedy. A dead `serve` leaves
an orphaned runner; the next `serve` or lazy boot reaps it by recorded pid and continues. A
foreign process holding port 8770 fails fast and names the port, device, and `lsof` remedy.

## Output contract

- stdout carries parseable data; stderr carries hints and errors. A broken stdout pipe exits 0.
- Exit codes: `0` ok, `1` driver or transport failure, `2` usage error.
- Snapshot header: `app=... device="..." os=... snapshot=@... refs=N settled=true complete=true
  reads=N elapsed_ms=N`. `settled=false` means the settle loop hit its cap — the tree is still
  usable. `complete=false` means `--max-depth` trimmed nodes below the printed depth.
- Refs look like `@<snapshot>:e<N>` and die with their snapshot; every action replies with the
  next snapshot, so a fresh ref set is always one call old at most.
- Errors name the next action: `STALE_REF` -> re-snapshot and retry; `AMBIGUOUS_TARGET` -> pick a
  ref with a firmer `native_id`, walk bounds via `--json`, or coordinate-tap; transport failures
  print the escalation checklist (serve running? cert trusted? same Wi-Fi? Mac awake?).

## Benchmark

```
cargo build --release --locked
python3 scripts/bench.py                     # ~25 min: 3 cold boots, 30 iterations, soak, scenario
python3 scripts/bench.py --quick             # ~5 min smoke run
python3 scripts/bench.py --compare bench-results/<old>.json
python3 scripts/bench.py --render bench-results/<run>.json   # rebuild the HTML only
```

It drives the release binary against the simulator and writes `bench-results/<stamp>.json` plus a
self-contained HTML report: 1280×720 slides, one question each, ready to screenshot. It covers cold
start, per-verb p50/p95, the CLI/driver/settle split, snapshot cost against tree size, typing
speed, settle rate, stale-ref refusals, error rate, tokens per snapshot, drift, runner memory and
CPU, and an end-to-end Settings task. The cold boots shut down the simulator; `--cold 0` skips
them.

## Physical iPhone

A physical device needs a signed runner, which npm cannot ship — clone this repo so `serve`
finds the source project (`drivers/ios/AgentMobileDriver.xcodeproj`), then:

```
agent-mobile serve "Lahfir's iPhone"
```

`xcodebuild` builds and signs with the project's team, installs the host app plus the runner, and
binds `0.0.0.0:8770` on the phone — the Mac reaches it over the same Wi-Fi. On first install iOS
refuses to launch the runner until the Developer App certificate is trusted on the phone:
Settings > General > VPN & Device Management > trust the certificate, then `serve` again — the
command detects the refusal and prints these steps. The phone shows "Automation Running" while
the driver is up. The driver URL is `http://<bonjour-host>.local:8770`; it lands in `state.json`
automatically, or pass it per call via `AGENT_MOBILE_URL`.

## Protocol

- Every call is `POST /<command>` with a JSON body, `Authorization: Bearer <token>`, and
  `X-Agent-Mobile-Version: 1`. Missing or wrong token -> 401.
- Commands: `status`; `launch {bundle_id}`; `terminate`;
  `snapshot {app?}`; `tap {ref} | {x,y}`; `type {text, ref?}`;
  `swipe {direction: up|down|left|right, ref?}`; `home`; `screenshot` (PNG base64).
- Response envelope: `{version, ok, command, elapsed_ms, data}` or
  `{version, ok:false, command, elapsed_ms, error:{code, message}}`. Error codes: STALE_REF,
  AMBIGUOUS_TARGET, BAD_REQUEST, UNKNOWN_COMMAND, UNAUTHORIZED, DRIVER_ERROR.
- `data` for snapshot and for every action: `app, snapshot_id, ref_count, complete, settled,
  reads, settle_ms, text, tree`. Every action returns the fresh post-action tree, so an action costs no
  extra round trip.
- Node: `role, name, value, ref_id, states, available_actions, native_id {kind:"ax_identifier",
  value}, bounds {x,y,width,height}, children`.
- Refs are per-snapshot and qualified: `@<snapshot_id>:eN`. Each action re-resolves its ref
  against the live tree by element type + identifier + label + frame (1 pt tolerance).
  Snapshot-id mismatch or no live match -> STALE_REF; more than one match -> AMBIGUOUS_TARGET.
  No self-healing, by design (`docs/research/07`, `docs/research/11`).
- Idle check: after every action the driver re-reads the tree until two consecutive tree hashes
  match, capped at 3 s; `settled` and `reads` report what happened.
- `Accept: text/plain` returns the compact listing instead of JSON, one line per named or
  interactive node: `@id:eN role "name" value="..." at=x,y size=WxH [states]`.

## Security

- Bearer token per session, generated on `serve`, written to a `0600` file, printed once.
- The Mac-to-phone hop is plain HTTP on the LAN; the phone binds all interfaces. Tunnel adapters
  (`drivers/ios/tunnel-cloudflared.sh`, ngrok, tailscale, ssh -R) add encryption on the way
  out, but the driver itself knows nothing about TLS.
- Whoever holds the URL and token has full UI control. No rate limit, expiry, allowlist, or
  audit log. Use a fresh token per session and only on a trusted Wi-Fi.

## Layout

- `src/`, `crates/core/` — the CLI and the shared core (contract, wire client, state, process).
- `npm/` — the darwin-only package: `run.js` shim, `install.js` postinstall verifier, bundled
  `bin/` + `runner/` produced by `scripts/sync-npm-version.sh`.
- `drivers/ios/` — the iOS driver: an Xcode project whose UI-test target hosts the HTTP
  server (`Driver/AgentMobileServer.swift`); `Host/` is the minimal app the runner attaches to.
  `am.sh` is a curl helper, `start-device.sh` starts the driver on a physical iPhone,
  `tunnel-cloudflared.sh` is the reference tunnel adapter.
- `docs/PRD.md` — the product requirements: contract, phases P1–P4 with experiment exit criteria,
  engineering practices, risks, and the reliability gate.
- `docs/research/` and `docs/experiments/` — the 13 research tracks with their synthesis, and the
  experiment record with verbatim output (Experiments 1–9) plus logs and screenshots.
