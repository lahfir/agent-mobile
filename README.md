# agent-mobile

Let an AI agent drive iOS and Android apps through a snapshot -> act loop, on the
simulator/emulator and on physical devices, with the driver exposed on a plain local HTTP port
so any tunnel can forward it. Sibling of agent-browser (web) and agent-desktop (native desktop).

## Status (2026-10-05)

- P1 iOS shipped: the Rust CLI (`agent-mobile`) wraps the proven XCUITest driver. Any verb
  lazy-starts a driver when none runs; `serve` runs one in the foreground. The npm package
  bundles a prebuilt simulator runner, so the iOS simulator path builds nothing on the user's
  Mac.
- P2 Android implemented on this branch: a Kotlin `AccessibilityService` driver plus a Rust
  ADB adapter serve the same protocol — cross-platform `devices`/`serve`, normalized
  `platform:id` device keys, a loopback bridge with one owned `adb forward`, and exact
  owned-resource cleanup. The emulator-CI unit and Experiment 10 (emulator proof plus a
  conditional physical-phone leg) are still pending, so treat Android support as
  implemented-but-not-yet-CI-proven.
- Probe evidence stands (`docs/experiments/RESULTS.md`, Experiments 1–9): agent-driven Calendar
  events on the iOS 26 simulator and on a physical iPhone 14 Pro over Wi-Fi, settled snapshots,
  STALE_REF refusing a mistap, one boot from a clean state.
- Not built (later phases): the reliability benchmark for Android (P3), the MCP wrapper (P4),
  the iOS in-app SDK, end-to-end encryption on the LAN hop.

## Install

The published npm package is macOS-only, Node >= 18 — it ships a prebuilt CLI binary and a
prebuilt iOS simulator runner, so installation compiles and downloads nothing for iOS. A
source checkout additionally supports the headless Android host on macOS arm64/x86_64 and
Linux x86_64 (the hosts the setup script declares).

```
npm i -g agent-mobile
agent-mobile --version
```

From a checkout instead (pins in `rust-toolchain.toml` and `Cargo.toml` apply):

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
agent-mobile devices                    # normalized rows: name, platform, id, kind, os, state
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
| `devices` | list iOS simulators/phones and Android AVD/USB/wireless targets |
| `serve <device>` | start a driver in the foreground; prints the session token once |
| `status` | active app, device, os, current snapshot id |
| `snapshot` | mint refs and print the accessibility tree |
| `tap <ref>` or `tap <x> <y>` | tap an element ref, or a point in the app frame |
| `type [<ref>] <text...>` | append text: a leading ref writes that field's value directly, otherwise keystrokes go to the focused field; use `--` before hyphen-leading text |
| `swipe <up\|down\|left\|right> [<ref>]` | swipe the app, or one element |
| `home` | go to the launcher/home; returns that tree |
| `back` | navigate back |
| `doubletap`, `pinch`, `hold`, `twofinger` | two-tap, zoom, press-and-hold, two-finger tap |
| `center notification` | open notifications from the system UI session |
| `launch <app_id>` | cold-start or restart an app (iOS bundle id / Android package); app data is kept |
| `screenshot [path]` | PNG to a file, or base64 to stdout |
| `stop` | terminate the active app; the driver stays up |
| `skills` | print the one-page agent guide |

Global flags: `--json` (raw JSON envelope instead of text), `--app <id>` (app selector for
`snapshot`/`serve` — iOS targets the bundle; Android `snapshot --app` only validates the
already-foreground package, so use `launch` to change apps), `--max-depth <n>` (trim the tree
client-side; `complete=false` follows), `--device <device>` (name, id, or `platform:id`;
remembered for later calls).

## Environment

| Variable | Effect |
|---|---|
| `AGENT_MOBILE_URL` | driver URL override for the call (e.g. a tunnel or a phone on the LAN) |
| `AGENT_MOBILE_TOKEN` | bearer token paired with the URL override |
| `AGENT_MOBILE_DRIVER_DIR` | iOS driver source override: an Xcode project dir or a `runner/` dir holding `*.xctestrun` |
| `AGENT_MOBILE_ANDROID_APK` | Android driver APK override; otherwise the checked-in Gradle wrapper builds the debug APK |
| `ANDROID_SDK_ROOT` / `ANDROID_AVD_HOME` | standard Android locations; the adapter also honors `ANDROID_HOME` and PATH |
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
builds the debug APK via the checked-in Gradle wrapper when `AGENT_MOBILE_ANDROID_APK`/a prior
artifact is absent, installs with `-r`, enables `Agent Mobile Driver` in Settings > Accessibility
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
nothing touches Play distribution — sideload/debug only.

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

It drives the release binary against the iOS simulator and writes `bench-results/<stamp>.json`
plus a self-contained HTML report: 1280×720 slides, one question each, ready to screenshot. It
covers cold start, per-verb p50/p95, the CLI/driver/settle split, snapshot cost against tree
size, typing speed, settle rate, stale-ref refusals, error rate, tokens per snapshot, drift,
runner memory and CPU, and an end-to-end Settings task. The cold boots shut down the simulator;
`--cold 0` skips them.

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

## Protocol

- Every call is `POST /<command>` with a JSON body, `Authorization: Bearer <token>`, and
  `X-Agent-Mobile-Version: 1`. Missing or wrong token -> 401. Protocol version is `1` on both
  platforms.
- Commands: `status`; `launch {bundle_id}`; `terminate`; `snapshot {app?}`; `tap {ref} | {x,y}`;
  `type {text, ref?}`; `swipe {direction: up|down|left|right, ref?}`; `home`;
  `screenshot` (PNG base64). `snapshot {app?}` on Android validates that the package is
  already foreground rather than retargeting — `launch` changes apps. On Android `launch`/
  `terminate` run through the host bridge with serial-scoped `cmd package`/`am force-stop`/
  `am start`; `terminate` refuses the driver itself, System UI, and the current launcher.
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
  No self-healing, by design (`docs/research/07`, `docs/research/11`).
- Idle check: after every action the driver re-reads the tree until two consecutive tree hashes
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
- `scripts/` — `setup-android-sdk.sh` (headless SDK install/check), `record-fixtures.sh`
  (fixture capture), `check-*.sh` source gates, `bench.py`.
- `docs/PRD.md` — the product requirements: contract, phases P1–P4 with experiment exit
  criteria, engineering practices, risks, and the reliability gate.
- `docs/research/` and `docs/experiments/` — the 13 research tracks with their synthesis, and
  the experiment record with verbatim output (Experiments 1–9) plus logs and screenshots.
