# agent-mobile

Let an AI agent drive iOS and Android apps through a snapshot -> act loop, on the
simulator/emulator and on physical devices, with the driver exposed on a plain local HTTP port
so any tunnel can forward it. Sibling of agent-browser (web) and agent-desktop (native desktop).

## Status (2026-10-18)

- P1 iOS shipped: the Rust CLI (`agent-mobile`) wraps the proven XCUITest driver. Any verb
  lazy-starts a driver when none runs; `serve` runs one in the foreground.
- P2 Android shipped: a Kotlin
  `AccessibilityService` driver plus a Rust ADB adapter serve the same protocol —
  cross-platform `devices`/`serve`, normalized `platform:id` device keys, a loopback
  bridge with one owned `adb forward`, and exact owned-resource cleanup. A headless
  Ubuntu emulator job gates the rail; the local live gate passed, and Experiment 10
  created a real Clock alarm.
- P3 shipped: the MCP server (`agent-mobile mcp`, 19 tools), three-read settle on both drivers,
  iOS system alerts in snapshots, `scroll_until_visible`, and the agent-task benchmark over MCP.
  Experiment 11 ran (results in PR #8). The physical-iPhone loopback bind is deferred.
- Probe evidence stands (Experiments 1–10, in the local experiment record): agent-driven Calendar
  events on the iOS 26 simulator and on a physical iPhone 14 Pro over Wi-Fi, plus the Android
  17 emulator alarm. Experiment 10's physical Android leg is explicitly not run because no
  authorized phone was available; its manual procedure remains in the experiment record.
- Not built (later phases): P4 package publication (npm and cargo publishing, MCP Registry listing), the iOS
  in-app SDK, end-to-end encryption on the LAN hop.

## Install

agent-mobile is not published to a package registry yet (P4). Install it from a checkout. The
pins in `rust-toolchain.toml` and `Cargo.toml` apply. On macOS, this builds the release CLI and
the prebuilt iOS simulator runner into `npm/`, then puts `agent-mobile` on your PATH (Node >= 18):

```
scripts/sync-npm-version.sh
(cd npm && npm link)
agent-mobile --version
```

The linked command runs the staged copy in `npm/bin/`, which does not rebuild itself. Run
`scripts/sync-npm-version.sh` again after every `git pull`. If a command that the docs list is
missing (for example `unrecognized subcommand 'mcp'`), the staged copy is old.

For the CLI alone (also the headless Android host on Linux x86_64), build it and use
`target/release/agent-mobile`:

```
cargo build --release --locked
```

For Android you need only the command-line SDK pieces — the setup does not install or require
Android Studio. It installs only missing pinned components and is idempotent (a second run
changes nothing):

```
scripts/setup-android-sdk.sh          # install missing pinned components
scripts/setup-android-sdk.sh --check  # verify without installing anything
```

Add the `ANDROID_HOME`/`ANDROID_SDK`/`ANDROID_SDK_ROOT`/`PATH` exports the script prints to
your shell profile; set `ANDROID_AVD_HOME` separately only when your AVDs live somewhere
nondefault.

## Quickstart

Any verb lazy-starts a driver when no session exists — it discovers devices on both platforms,
picks the default (an iPhone-named iOS simulator, then any iOS simulator, then an iOS
physical device, then a ready Android target, then any Android target),
saves the session under `~/.agent-mobile/`, and runs the verb. The first call can take a
minute; progress prints to stderr.

```
agent-mobile devices                    # rows: key, name, platform, kind, os, state, serving
agent-mobile snapshot                   # lazy-boots, prints the accessibility tree
agent-mobile launch com.apple.mobilecal # iOS bundle id — Android takes a package name
agent-mobile tap @1abc:e7               # act on a ref from the last snapshot
agent-mobile type "Dentist tomorrow"    # type into the focused field
agent-mobile screenshot shot.png        # PNG file; omit the path for base64 on stdout
agent-mobile skills                     # the one-page agent guide
```

To run the driver explicitly (one session total, foreground, Ctrl-C cleans up):

```
agent-mobile serve "iPhone 17 Pro Max"           # iOS display name
agent-mobile serve "ios:YOUR-UDID"               # iOS stable key
agent-mobile serve android:avd:agent-mobile-api37  # Android AVD stable key
agent-mobile serve "android:YOUR-SERIAL"         # Android USB/wireless serial
```

`<device>` accepts a display name, a raw id, or the collision-free `platform:id` key. When two
devices share a name the CLI refuses to guess — pick by key.

## Commands

| Command | What it does |
|---|---|
| `devices [--platform <platform>] [--kind <kind>] [--state <state>]` | list iOS simulators/phones and Android AVD/USB/wireless targets; filters combine with AND |
| `serve <device>` | start a driver in the foreground; prints the session token once |
| `status` | active app, device, os, current snapshot id |
| `snapshot` | mint refs and print the accessibility tree |
| `tap <ref>` or `tap <x> <y>` | tap an element ref, or a point in the app frame |
| `type [<ref>] <text...>` | append text: a leading ref writes that field's value directly, otherwise keystrokes go to the focused field; use `--` before hyphen-leading text |
| `swipe <up\|down\|left\|right> [<ref>]` | swipe the app, or one element |
| `scroll_until_visible <name> [up\|down\|left\|right]` | swipe until a node with that exact name appears (at most 15 swipes) |
| `home` | go to the launcher/home; returns that tree |
| `back` | navigate back |
| `doubletap`, `pinch`, `hold`, `twofinger` | two-tap, zoom, press-and-hold, two-finger tap |
| `center notification` | open notifications from the system UI session |
| `launch <app_id>` | cold-start or restart an app (iOS bundle id / Android package); app data is kept |
| `screenshot [path]` | PNG to a file, or base64 to stdout |
| `stop` | terminate the active app; the driver stays up |
| `release` | stop the running driver session and owned resources; the device stays up |
| `skills` | print the one-page agent guide |
| `mcp` | serve the tools over MCP on stdio (see MCP server) |

Global flags: `--json` (raw JSON envelope instead of text), `--app <id>` (app selector for
`snapshot`/`serve` — iOS targets the bundle; Android `snapshot --app` only validates the
already-foreground package, so use `launch` to change apps), `--max-depth <n>` (trim the tree
client-side; `complete=false` follows), `--device <device>` (name, id, or `platform:id`;
remembered for later calls).

## MCP server

`agent-mobile mcp` is a stdio MCP server (newline-delimited JSON-RPC). Its tools and argument
definitions come from the same registry as the CLI. One server process owns one active serve at a
time; each tool that talks to a device accepts optional `device` to select its target for that call without changing the
remembered default. A different device cannot take over while another holds the single serve; call
`release` first.

```sh
claude mcp add agent-mobile -- agent-mobile mcp --device "iPhone 16"
```

Other clients accept a generic `mcpServers` block:

```json
{ "mcpServers": { "agent-mobile": { "command": "agent-mobile", "args": ["mcp", "--device", "iPhone 16"] } } }
```

Tools (19): `devices`, `status`, `snapshot`, `tap`, `type`, `swipe`, `home`, `doubletap`, `pinch`,
`hold`, `back`, `twofinger`, `center`, `launch`, `screenshot`, `stop`, `release`, `skills`,
`scroll_until_visible`. `mcp` and `serve` are CLI commands, not tools. `launch` takes `app_id` (an
iOS bundle id or Android package). `tap`, `doubletap`, and `hold` take a `target` object shaped as
`{"ref": "@snapshot:e1"}` or `{"x": 12, "y": 34}`. `screenshot` takes no arguments and returns an image.

Data replies include `structuredContent` governed by output schemas for device-list, status,
snapshot/action, and stop (`terminate`) replies. Their text content keeps the framed compact listing; snapshot metadata
is structured separately so the listing is not duplicated as JSON. Argument faults return an
`isError` tool result naming the field, problem, and fix. Unknown tools and malformed protocol
requests remain JSON-RPC errors.

What an agent must know:

- Refs die after every action. On `STALE_REF`, run `snapshot` again and use the new refs.
- The first call while a driver boots returns "driver is starting; call again" within about 45 s.
  The boot continues in the background, so call again.
- A call that times out may already have run. `snapshot` before you repeat the action.
- `AGENT_MOBILE_MCP_CALL_SECS` lowers the time limit for one call (default and maximum: 45 s).

Smoke check, listing the tools:

```sh
npx @modelcontextprotocol/inspector@2.11.0 --cli agent-mobile mcp --method tools/list
```

The Inspector does not pass your shell environment to the server. Give each variable the server needs with `-e`, for example `-e ANDROID_HOME="$ANDROID_HOME"`.

## Environment

| Variable | Effect |
|---|---|
| `AGENT_MOBILE_URL` | driver URL override for the call (e.g. a tunnel or a phone on the LAN) |
| `AGENT_MOBILE_TOKEN` | bearer token paired with the URL override |
| `AGENT_MOBILE_DRIVER_DIR` | iOS driver source override: an Xcode project dir or a `runner/` dir holding `*.xctestrun` |
| `AGENT_MOBILE_ANDROID_APK` | Android driver APK override; otherwise the checked-in Gradle wrapper builds the debug APK |
| `ANDROID_HOME` / `ANDROID_SDK_ROOT` | SDK location, in that order; then the platform default, then PATH |
| `AGENT_MOBILE_BOOT_BUDGET_SECS` | lazy-boot deadline in seconds (default 240); raise it past 600 before the first Android boot, which can run a cold Gradle build |
| `AGENT_MOBILE_REPO_ROOT` | checkout root that holds `drivers/android`; needed only outside a checkout |
| `AGENT_MOBILE_MCP_CALL_SECS` | time limit for one MCP tool call (default and maximum: 45 s) |
| `TEST_RUNNER_AGENT_MOBILE_PORT` / `_TOKEN` / `_BIND` | iOS driver-side env, set through xcodebuild's `TEST_RUNNER_` prefix (see `drivers/ios/am.sh`) |

## Sessions and state

Everything lives under `~/.agent-mobile/`:

- `state.json` — session rows keyed by `platform:id` (legacy iOS name keys still resolve):
  driver URL, serve pid, runner pid or Android serial/forward/bridge/APK metadata, token
  filename, and the remembered default device. Never holds a token value.
- `tokens/` — one `0600` file per session; the token itself.
- `driver-<device>.log` — the driver-side log, written `0600` because it can echo env.
- `serve.lock`, `boot.lock` — one serve at a time; one lazy boot at a time.

A second `serve` on a live device reports the URL, the pid, and the remedy. A dead `serve`
leaves an orphaned runner (iOS) or forward row (Android); the next serve reaps exactly the
recorded resource — the recorded iOS runner PID or Android serial+forward port, never a broad
port or process sweep. On iOS a foreign process holding port 8770 fails
fast and names the `lsof` remedy; Android uses ephemeral loopback ports so there is nothing to
conflict.

## Android bring-up and cleanup

The first Android `serve` boots the AVD if shutdown (or reuses a running/physical target),
uses `AGENT_MOBILE_ANDROID_APK` as supplied when set, otherwise invokes the checked-in Gradle
wrapper — which incrementally validates source/build inputs and skips unchanged tasks even when
a prior APK artifact exists — then installs with `-r`, enables `Agent Mobile Driver` in
Settings > Accessibility
(preserving every other enabled service), provisions a fresh bearer through the app's
`ContentProvider`, creates one owned `adb forward`, and exposes a loopback bridge. It never
installs SDK packages at session startup — run `scripts/setup-android-sdk.sh` first.

Stopping a serve removes only what it owns — the bridge, the exact `adb forward`, and the
session state/token. The emulator keeps running, the APK stays installed, and the
accessibility service stays enabled.

Physical Android: enable Developer options and USB debugging, authorize the computer at the
device prompt, then `adb devices` should list the serial. For official wireless debugging
(Android 11+): `adb pair <ip>:<pair-port>` then `adb connect <ip>:<connect-port>` — the ports
differ and rotate; the pairing persists until revoked. Authorized adb is one consent boundary;
nothing touches Play distribution — sideload/debug only. The adapter supports these serials,
but the Experiment 10 physical-phone leg awaits an available authorized device.

Accessibility approval is the second consent boundary:

- The driver must be explicitly enabled as the `Agent Mobile Driver` service in Settings >
  Accessibility.
- Android 13+ Restricted Settings can block accessibility for sideloaded apps: App Info >
  three-dot menu > "Allow restricted settings", then Settings > Accessibility >
  Agent Mobile Driver.
- Android 17 Advanced Protection can block sideload/USB debugging and restrict non-assistive
  automation — unsupported while the OS blocks it; there is no bypass.

### Android troubleshooting

| Symptom | Remedy |
|---|---|
| `adb` missing / no SDK | `scripts/setup-android-sdk.sh --check`; add the printed exports |
| APK missing | `drivers/android/gradlew -p drivers/android :app:assembleDebug --no-daemon`, or set `AGENT_MOBILE_ANDROID_APK` |
| `unauthorized` row | accept the USB debugging prompt on the device |
| `offline` row | `adb reconnect` or replug, then retry |
| service won't bind | enable Restricted Settings (above) then the accessibility toggle |
| duplicate device names | select with the `platform:id` key |
| AVD boot timeout | inspect `~/.agent-mobile/driver-*.log` |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE` | signature mismatch — `adb -s <serial> uninstall com.lahfir.agentmobile.driver` if you choose (removes app data; never done automatically) |

## Output contract

- stdout carries parseable data; stderr carries hints and errors. A broken stdout pipe exits 0.
- Exit codes: `0` ok, `1` driver or transport failure, `2` usage error.
- Snapshot header: `app=... device="..." os=... snapshot=@... refs=N settled=true complete=true
  reads=N elapsed_ms=N`. `settled=false` means the settle loop hit its cap — the tree is still
  usable. `complete=false` means the tree is
  incomplete — `--max-depth` trimmed nodes client-side, or the driver hit a traversal cap, a
  cycle, or an unavailable child. Ref actions on an incomplete tree fail closed because ref
  uniqueness can't be proven.
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
python3 scripts/bench.py --device android:avd:<name>   # same suite against Android
python3 scripts/bench.py --compare bench-results/<old>.json
python3 scripts/bench.py --render bench-results/<run>.json   # rebuild the HTML only
python3 scripts/bench.py --agent --platforms android --seeds 5 --servers <local.json>  # agent tasks over MCP
```

It drives the release binary against the iOS simulator (or an Android emulator via
`--device`) and writes `bench-results/<stamp>.json`
plus a self-contained HTML report: 1280×720 slides, one question each, ready to screenshot. It
covers cold start, per-verb p50/p95, the CLI/driver/settle split, snapshot cost against tree
size, typing speed, settle rate, stale-ref refusals, error rate, tokens per snapshot, drift,
runner memory and CPU, and an end-to-end Settings task. The cold boots shut down the simulator;
`--cold 0` skips them.

`--agent` runs the agent tasks: a fixed model drives each task through one MCP server per run,
and a host-side check that uses no tool under test decides success. `--tasks` narrows the task
list. `--servers` names a local, untracked JSON file with the comparison servers' launch
commands; without it only agent-mobile runs.

## Physical iPhone

A physical device needs a signed runner, which npm cannot ship — clone this repo so `serve`
finds the source project (`drivers/ios/AgentMobileDriver.xcodeproj`), then:

```
agent-mobile serve "Lahfir's iPhone"
```

`xcodebuild` builds and signs with the project's team, installs the host app plus the runner,
and binds `0.0.0.0:8770` on the phone — the Mac reaches it over the same Wi-Fi. On first
install iOS refuses to launch the runner until the Developer App certificate is trusted on the
phone: Settings > General > VPN & Device Management > trust the certificate, then `serve`
again — the command detects the refusal and prints these steps. The phone shows "Automation
Running" while the driver is up. The driver URL is `http://<bonjour-host>.local:8770`; it
lands in `state.json` automatically, or pass it per call via `AGENT_MOBILE_URL`.

### iPhone troubleshooting

| Cause | What to do |
| --- | --- |
| The iPhone has no free app slot for this signing profile | Delete apps signed by your personal team that you no longer need in Settings > General > VPN & Device Management, then retry. |
| The iPhone refused to install the driver: this Xcode likely does not support the iPhone's iOS version | Update Xcode to a version that supports the iPhone's iOS, then retry. If Xcode is current, free an app slot for your personal team. |
| The iPhone does not trust the developer certificate | In Settings > General > VPN & Device Management, trust the developer app, then retry. |
| The iPhone is locked | Unlock it and keep it unlocked until the driver answers, then retry. |
| Developer Mode is off on the iPhone | Turn on Settings > Privacy & Security > Developer Mode, restart the iPhone, then retry. |
| Xcode has no signing team for the driver | Sign in under Xcode > Settings > Accounts. Open the driver's `AgentMobileDriver.xcodeproj`, choose your team under Signing & Capabilities for every target, then retry. |

## Protocol

- Every call is `POST /<command>` with a JSON body, `Authorization: Bearer <token>`, and
  `X-Agent-Mobile-Version: 1`. Missing or wrong token -> 401. Protocol version is `1` on both
  platforms.
- Commands: `status`; `launch {bundle_id}`; `terminate`; `snapshot {app?}`; `tap {ref} | {x,y}`;
  `type {text, ref?}`; `swipe {direction: up|down|left|right, ref?}`; `home`;
  `back`; `doubletap`; `hold`; `pinch`; `twofinger`; `center`; `screenshot` (PNG base64).
  `scroll_until_visible` has no wire command: the host loops `swipe` and `snapshot`. `snapshot {app?}` on Android validates that the package is
  already foreground rather than retargeting — `launch` changes apps. On Android `launch`/
  `terminate` run through the host bridge with serial-scoped `cmd package`/`am force-stop`/
  `am start`; `terminate` refuses the driver itself, System UI, and the current launcher.
  `terminate` targets the package seen by its authenticated status preflight; foreground
  observation and force-stop are not atomic, and a later foreground change does not retarget
  the request.
  If System UI wedges the foreground, `back` is the first recovery move — judge the returned
  snapshot; `launch` is only a cold-restart fallback and loses transient app state.
- Response envelope: `{version, ok, command, elapsed_ms, data}` or
  `{version, ok:false, command, elapsed_ms, error:{code, message}}`. Error codes: STALE_REF,
  AMBIGUOUS_TARGET, BAD_REQUEST, UNKNOWN_COMMAND, UNAUTHORIZED, DRIVER_ERROR.
- `data` for snapshot and for every action: `app, snapshot_id, ref_count, complete, settled,
  reads, settle_ms, text, tree`. Every action returns the fresh post-action tree, so an action
  costs no extra round trip.
- Node: `role, name, value, ref_id, states, available_actions, native_id {kind, value},
  bounds {x,y,width,height}, children`. `native_id.kind` is `ax_identifier` on iOS and
  `resource_id` on Android.
- Refs are per-snapshot and qualified: `@<snapshot_id>:eN`. Each action re-resolves its ref
  against the live tree by element type + identifier + label + frame (1 pt tolerance).
  Snapshot-id mismatch or no live match -> STALE_REF; more than one match -> AMBIGUOUS_TARGET.
  No self-healing, by design.
- Idle check: after every action the driver re-reads the tree until three consecutive tree hashes
  match, capped at 3 s; `settled` and `reads` report what happened.
- `Accept: text/plain` returns the compact listing instead of JSON, one line per named or
  interactive node: `@id:eN role "name" value="..." at=x,y size=WxH [states]`.

## Security

- Bearer token per session, generated on `serve`, written to a `0600` file, printed once.
- iOS physical: the Mac-to-phone hop is plain HTTP on the LAN; the phone binds all interfaces.
  Tunnel adapters (`drivers/ios/tunnel-cloudflared.sh`, ngrok, tailscale, ssh -R) add
  encryption on the way out, but the driver itself knows nothing about TLS.
- Android: the agent endpoint is loopback-only — the bridge and the owned `adb forward` keep
  device traffic on the host, behind an authorized adb transport.
- Whoever holds the URL and token has full UI control. No rate limit, expiry, allowlist, or
  audit log. Use a fresh token per session — on a trusted Wi-Fi for a physical iPhone, and on
  a host you already trust for Android's authorized-adb transport.

## Layout

- `src/`, `crates/core/` — the CLI and the shared core (contract, wire client, state, process).
- `crates/android/` — the Android host adapter: ADB discovery/boot/install/provision/forward,
  the loopback HTTP bridge, and `AndroidAdapter`/`AndroidSession`.
- `drivers/android/` — the Android driver: a Kotlin `AccessibilityService` serving the same
  protocol, a `ProvisionProvider` for token minting, and its Gradle project (`gradlew` wrapper).
- `npm/` — the darwin-only package: `run.js` shim, `install.js` postinstall verifier, bundled
  `bin/` + `runner/` produced by `scripts/sync-npm-version.sh`.
- `drivers/ios/` — the iOS driver: an Xcode project whose UI-test target hosts the HTTP
  server (`Driver/AgentMobileServer.swift`); `Host/` is the minimal app the runner attaches to.
  `am.sh` is a curl helper and `tunnel-cloudflared.sh` is the reference tunnel adapter.
- `scripts/` — `setup-android-sdk.sh` (headless SDK install/check),
  `ci-boot-android-emulator.sh` (bounded CI lifecycle), `record-fixtures.sh` (fixture capture),
  `check-*.sh` source gates, `bench.py`.
- `docs/PRD.md` — the product requirements: contract, phases P1–P4 with experiment exit
  criteria, engineering practices, risks, and the reliability gate.
- `docs/research/` and `docs/experiments/` (local only, gitignored) — the research tracks and
  the experiment record. A clone does not have them.
