# agent-mobile

Let an AI agent use iOS and Android apps. The agent reads the screen as an accessibility tree, acts on a ref, and gets the next tree back.

Works on the iOS simulator, the Android emulator, and real phones.

```
agent-mobile launch com.apple.Preferences   # boots a simulator, opens Settings, prints the tree
agent-mobile tap @1abc:e7                   # taps a ref from that tree, prints the next tree
```

## Quick start

1. Install from a checkout (macOS):

   ```sh
   scripts/sync-npm-version.sh   # builds the CLI, the iOS simulator runner, and the Android APK into npm/
   (cd npm && npm link)          # puts agent-mobile on your PATH
   agent-mobile --version
   ```

2. Add the MCP server to your agent (other agents: [MCP](#use-with-an-agent-mcp)):

   ```sh
   claude mcp add agent-mobile -- agent-mobile mcp
   ```

3. Ask the agent: "Open Settings on the iPhone simulator and turn on Dark Mode."

The first call boots a device and starts the driver. It can take a minute. Later calls are fast.

## Install

Quick start step 1 is the full install on macOS. The pins in `rust-toolchain.toml` and `Cargo.toml` apply.

`agent-mobile` runs the copy in `npm/bin/`. It does not rebuild itself. Run `scripts/sync-npm-version.sh` again after each `git pull`.

**CLI only, or a headless Android host on Linux x86_64:**

```sh
cargo build --release --locked   # binary: target/release/agent-mobile
```

**Android SDK.** You need the command-line SDK only, not Android Studio. The script installs only the missing pinned parts. A second run changes nothing.

```sh
scripts/setup-android-sdk.sh          # install the missing parts
scripts/setup-android-sdk.sh --check  # check only, install nothing
```

Add the `ANDROID_HOME`, `ANDROID_SDK`, `ANDROID_SDK_ROOT`, and `PATH` exports that the script prints to your shell profile. Set `ANDROID_AVD_HOME` only if your AVDs are not in the default folder.

**If a command is missing** (for example `unrecognized subcommand 'mcp'`), your copy in `npm/` is old. Run the sync script again. If `agent-mobile` points at a deleted `run.js`, your link is from an old layout: run `(cd npm && npm link)` again. The sync script warns about this link.

## Use with an agent (MCP)

`agent-mobile mcp` is an MCP server on stdio. It has the same commands as the CLI.

Claude Code: see Quick start step 2.

**opencode** (`~/.config/opencode/opencode.json`):

```json
{ "mcp": { "agent-mobile": { "type": "local", "command": ["agent-mobile", "mcp"], "environment": { "AGENT_MOBILE_HEADED": "1" } } } }
```

**Other clients:**

```json
{ "mcpServers": { "agent-mobile": { "command": "agent-mobile", "args": ["mcp"] } } }
```

Add `--device "iPhone 16"` after `mcp` to choose the default device.

**Install the agent guide.** `skills/agent-mobile/SKILL.md` tells the agent how to start, how refs work, and what to do for each error. Put it where your agent looks for skills:

```sh
mkdir -p ~/.agents/skills/agent-mobile
agent-mobile skills > ~/.agents/skills/agent-mobile/SKILL.md
```

opencode and Claude Code also read `~/.config/opencode/skills/` and `~/.claude/skills/`. Run the command again after you update agent-mobile.

### Tools

19 tools: `devices`, `status`, `snapshot`, `tap`, `type`, `swipe`, `home`, `doubletap`, `pinch`, `hold`, `back`, `twofinger`, `center`, `launch`, `screenshot`, `stop`, `release`, `skills`, `scroll_until_visible`.

- `mcp` and `serve` are CLI commands only.
- `launch` takes `app_id`: an iOS bundle id or an Android package.
- `tap`, `doubletap`, and `hold` take `target`: `{"ref": "@snapshot:e1"}` or `{"x": 12, "y": 34}`.
- `screenshot` takes no arguments and returns an image.
- Each device tool takes an optional `device`. It picks the device for that call only and does not change the default.

### Rules and replies

- One server runs one driver at a time. To use another device, call `release` first.
- The first call while a driver boots returns "driver is starting; call again" within about 45 s. The boot continues, so call again.
- A call that times out can still run on the device. Call `snapshot` before you repeat it.
- `AGENT_MOBILE_MCP_CALL_SECS` lowers the time limit per call. The default and the maximum is 45 s.
- Data replies have `structuredContent` that matches an output schema: device list, status, snapshot or action, and stop (`terminate`).
- The text content keeps the framed compact tree. The snapshot metadata is in `structuredContent`, so the tree is not sent twice.
- A bad argument returns a tool result with `isError` that names the field, the problem, and the fix.
- An unknown tool or a malformed request returns a JSON-RPC error.

### Test the server

```sh
npx @modelcontextprotocol/inspector@2.11.0 --cli agent-mobile mcp --method tools/list
```

The Inspector does not pass your shell environment to the server. Pass each variable with `-e`, for example `-e ANDROID_HOME="$ANDROID_HOME"`.

## Use from the shell (CLI)

```sh
agent-mobile devices                    # list devices: key, name, platform, kind, os, state, serving
agent-mobile snapshot                   # boots a device if needed, prints the tree
agent-mobile launch com.apple.mobilecal # iOS bundle id; Android takes a package name
agent-mobile tap @1abc:e7               # act on a ref from the last tree
agent-mobile type "Dentist tomorrow"    # type into the focused field
agent-mobile screenshot shot.png        # PNG file; no path prints base64
agent-mobile skills                     # print the agent guide
```

**Any command starts a driver** if none runs. It finds devices on both platforms, picks one, saves the session in `~/.agent-mobile/`, and runs the command. The first call can take a minute. Progress goes to stderr.

The default device is the first match in this order:

1. an iOS simulator with "iPhone" in its name
2. any iOS simulator
3. a physical iPhone
4. a ready Android target
5. any Android target

**Choose a device** with `--device`. It takes a display name, a raw id, or a `platform:id` key, and it is remembered. If two devices have the same name, the CLI does not guess: use the key.

**Run the driver in the foreground** (one session at a time; Ctrl-C cleans up):

```sh
agent-mobile serve "iPhone 17 Pro Max"            # iOS display name
agent-mobile serve "ios:YOUR-UDID"                # iOS key
agent-mobile serve android:avd:agent-mobile-api37 # Android AVD key
agent-mobile serve "android:YOUR-SERIAL"          # Android USB or wireless serial
```

## Commands

| Command | What it does |
|---|---|
| `devices [--platform <platform>] [--kind <kind>] [--state <state>]` | List iOS simulators and phones, and Android AVD, USB, and wireless targets. Filters combine with AND. `--platform`: `ios`, `android`. `--kind`: `simulator`, `device`, `emulator`, `usb`, `wireless`. `--state` is lowercase. |
| `serve <device>` | Run a driver in the foreground. Prints the session token once. |
| `status` | Show the active app, device, OS, and current snapshot id. |
| `snapshot` | Read the screen as a tree with refs. |
| `tap <ref>` or `tap <x> <y>` | Tap a ref, or a point in the app frame. |
| `type [<ref>] <text...>` | Add text. With a ref, it writes that field's value. Without a ref, it types into the focused field. Put `--` before text that starts with `-`. |
| `swipe <up\|down\|left\|right> [<ref>]` | Swipe the app, or one element. |
| `scroll_until_visible <name> [up\|down\|left\|right]` | Swipe until a node with that exact name shows. At most 15 swipes. |
| `home` | Go to the home screen. Returns its tree. |
| `back` | Go back one screen. |
| `doubletap`, `pinch`, `hold`, `twofinger` | Double-tap, zoom, press and hold, two-finger tap. |
| `center notification` | Open the notifications. |
| `launch <app_id>` | Start or restart an app (iOS bundle id or Android package). The app keeps its data. |
| `screenshot [path]` | Save a PNG, or print base64. |
| `stop` | Close the active app. The driver keeps running. |
| `release` | Stop the driver session and what it owns. The device keeps running. |
| `skills` | Print the agent guide, `skills/agent-mobile/SKILL.md`. |
| `mcp` | Run the MCP server on stdio. |

Run `agent-mobile <command> --help` for the full rules of a command.

**Global flags:**

| Flag | Effect |
|---|---|
| `--json` | Print the raw JSON reply instead of text. |
| `--device <device>` | Name, id, or `platform:id` key. Remembered for later calls. |
| `--app <id>` | App for `snapshot` and `serve`. On iOS it targets that app. On Android, `snapshot --app` only checks that the app is in front; use `launch` to change apps. |
| `--max-depth <n>` | Cut the tree at depth n. The reply then shows `complete=false`. |

## Output

- stdout has the data. stderr has hints and errors. A closed stdout pipe exits 0.
- Exit codes: `0` ok, `1` driver or connection failure, `2` usage error.
- Refs look like `@<snapshot>:e<N>`. They stop working after the next action. Each action returns the next tree, so new refs are never more than one call old.

The snapshot header:

```
app=... device="..." os=... snapshot=@... refs=N settled=true complete=true reads=N elapsed_ms=N
```

- `settled=false`: the screen did not stop moving within the time limit. The tree is still usable.
- `complete=false`: the tree is cut short. `--max-depth` cut it, or the driver hit a traversal limit, a cycle, or a child it could not read. Ref actions on this tree fail, because the driver cannot prove that a ref is unique.

Each error names the next step:

| Error | Next step |
|---|---|
| `STALE_REF` | Take a new snapshot and retry. |
| `AMBIGUOUS_TARGET` | Use a ref with a firmer `native_id`, read the bounds with `--json`, or tap x,y. |
| Connection failure | The CLI prints a checklist: is serve running, is the certificate trusted, same Wi-Fi, is the Mac awake? |

## Configuration

| Variable | Effect |
|---|---|
| `AGENT_MOBILE_HEADED` | Set to `1` to show the device window when agent-mobile boots a device (Simulator window on iOS, emulator window on Android). An emulator that already runs without a window stays that way. |
| `AGENT_MOBILE_URL` | Driver URL for the call, for example a tunnel or a phone on the LAN. |
| `AGENT_MOBILE_TOKEN` | Token for `AGENT_MOBILE_URL`. |
| `AGENT_MOBILE_MCP_CALL_SECS` | Time limit for one MCP tool call. Default and maximum: 45 s. |
| `AGENT_MOBILE_BOOT_BUDGET_SECS` | Time limit for a lazy boot. Default 240 s. Set it above 600 before the first Android boot from a checkout, because that boot can run a cold Gradle build. |
| `AGENT_MOBILE_DRIVER_DIR` | iOS driver source: an Xcode project folder, or a `runner/` folder with a `*.xctestrun` file. |
| `AGENT_MOBILE_ANDROID_APK` | Android driver APK. If not set: the APK in the npm package (`android/`), then a Gradle build of the debug APK in the checkout. |
| `AGENT_MOBILE_REPO_ROOT` | Checkout root that holds `drivers/android`. Needed only outside a checkout. |
| `ANDROID_HOME` / `ANDROID_SDK_ROOT` | SDK location, in that order. Then the platform default, then PATH. |
| `TEST_RUNNER_AGENT_MOBILE_PORT` / `_TOKEN` / `_BIND` | iOS driver settings, passed through xcodebuild's `TEST_RUNNER_` prefix (see `drivers/ios/am.sh`). |

## Android

**First start.** The first Android session does these steps:

1. Boots the AVD if it is off. A running emulator or a physical phone is used as is.
2. Finds the driver APK: `AGENT_MOBILE_ANDROID_APK`, then the APK in the npm package (`android/agent-mobile-driver.apk`), then a Gradle build from the checkout.
3. Installs it with `-r`. It skips the install when the device has the same APK (same MD5), because a reinstall restarts the driver and its accessibility link.
4. Turns on `Agent Mobile Driver` in Settings > Accessibility. Other services stay on.
5. Makes a new token through the app's `ContentProvider`.
6. Makes one `adb forward` and a loopback bridge.

It never installs SDK packages during a session. Run `scripts/setup-android-sdk.sh` first.

**Cleanup.** A stopped session removes only what it made: the bridge, its `adb forward`, and its state and token. The emulator keeps running. The APK stays installed. The accessibility service stays on.

**Physical phone:**

1. Turn on Developer options and USB debugging.
2. Accept the computer on the phone prompt.
3. `adb devices` must list the serial.

For wireless debugging (Android 11+): `adb pair <ip>:<pair-port>`, then `adb connect <ip>:<connect-port>`. The two ports are different and they change. The pairing stays until you revoke it.

Authorized adb is the first consent. Accessibility is the second:

- Turn on the `Agent Mobile Driver` service in Settings > Accessibility.
- Android 13+ "Restricted settings" can block accessibility for sideloaded apps. Go to App Info > three-dot menu > "Allow restricted settings", then Settings > Accessibility > Agent Mobile Driver.
- Android 17 Advanced Protection can block sideloading and USB debugging, and can limit automation. agent-mobile does not work while the OS blocks it. There is no bypass.

### Android troubleshooting

| Problem | Fix |
|---|---|
| `adb` missing, or no SDK | `scripts/setup-android-sdk.sh --check`, then add the printed exports. |
| APK missing | `drivers/android/gradlew -p drivers/android :app:assembleDebug --no-daemon`, or set `AGENT_MOBILE_ANDROID_APK`. |
| Row shows `unauthorized` | Accept the USB debugging prompt on the phone. |
| Row shows `offline` | `adb reconnect` or plug the cable in again, then retry. |
| Service does not start | Allow restricted settings (above), then turn on the accessibility switch. |
| Two devices with the same name | Use the `platform:id` key. |
| AVD boot timeout | Read `~/.agent-mobile/driver-*.log`. |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE` | The signatures differ. You can run `adb -s <serial> uninstall com.lahfir.agentmobile.driver`. This deletes the app's data, so agent-mobile never does it for you. |

## Physical iPhone

A real iPhone needs a signed runner, and npm cannot ship one. Clone this repo so `serve` finds `drivers/ios/AgentMobileDriver.xcodeproj`, then run:

```sh
agent-mobile serve "My iPhone"
```

- `xcodebuild` builds and signs with the project's team, installs the host app and the runner, and binds `0.0.0.0:8770` on the phone. The Mac reaches it over the same Wi-Fi.
- On the first install, iOS does not launch the runner until you trust the developer certificate: Settings > General > VPN & Device Management > trust it, then run `serve` again. `serve` finds this error and prints these steps.
- The driver URL is `http://<bonjour-host>.local:8770`. It goes into `state.json` by itself. You can also pass it per call with `AGENT_MOBILE_URL`.

### iPhone troubleshooting

| Cause | Fix |
|---|---|
| No free app slot for this signing profile | Delete apps signed by your personal team that you do not need (Settings > General > VPN & Device Management), then retry. |
| The iPhone does not install the driver; this Xcode probably does not support its iOS version | Update Xcode, then retry. If Xcode is current, free an app slot for your personal team. |
| The iPhone does not trust the developer certificate | Trust the developer app in Settings > General > VPN & Device Management, then retry. |
| The iPhone is locked | Unlock it. Keep it unlocked until the driver answers, then retry. |
| Developer Mode is off | Turn on Settings > Privacy & Security > Developer Mode, restart the iPhone, then retry. |
| Xcode has no signing team for the driver | Sign in under Xcode > Settings > Accounts. Open `AgentMobileDriver.xcodeproj`, choose your team under Signing & Capabilities for each target, then retry. |

## Limitations

- A physical iPhone needs a Mac with Xcode running for the whole session, a paired phone, Developer Mode, and a trusted developer certificate. No cable is needed after pairing.
- The phone shows the "Automation Running" cover for the whole session. It cannot be removed.
- The developer certificate trust can lapse after about an hour. Trust it again, or use a paid developer profile.
- On a physical iPhone, the driver listens on all interfaces over plain HTTP. The token is the only gate on that link.
- Canvas UIs, games, and opaque WebViews have no tree.
- Android: the driver app is sideload only. Play policy bans this use of the accessibility API.
- Physical Android needs authorized USB or wireless ADB, the accessibility switch, and on some releases Restricted Settings consent.

## How it works

```
 AI agent ──MCP (stdio)──┐
                         ├──► agent-mobile (Rust CLI) ──HTTP + JSON + token──┬──► iOS driver ──► iOS apps
 You ──────CLI───────────┘                                                    └──► Android driver ──► Android apps
```

- **iOS driver** (`drivers/ios/`): an XCUITest runner with an HTTP server. It reads the tree with `XCUIElement` snapshots.
- **Android driver** (`drivers/android/`): an APK with an `AccessibilityService` and an HTTP server.
- **Settle:** after each action, the driver reads the tree again until three reads in a row match, for at most 3 s. `settled` and `reads` report the result.
- **Refs:** each action finds its ref again on the live tree. If the element is gone, you get `STALE_REF`. If more than one element matches, you get `AMBIGUOUS_TARGET`. The driver never guesses.

Internals, the wire protocol, sessions, and security: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

To contribute, see [CONTRIBUTING.md](CONTRIBUTING.md).
