# Contributing

## Setup

```
git config core.hooksPath .githooks
```

Before each commit that touches Rust, the hook runs the source rules, `cargo fmt`, `cargo clippy`, and `cargo test`. CI also runs the script tests and `cargo deny`.

The toolchain is Rust 1.89.0, pinned in `rust-toolchain.toml`, with clippy and rustfmt. The license is Apache-2.0.

Dependencies stay small: `clap`, `serde`, `serde_json`, and blocking HTTP. The project has no async runtime.

## Repo layout

| Path | Contents |
|---|---|
| `src/` | The CLI. Platform branches live in `src/platform/`, which normalizes the platforms. |
| `src/surface/verbs.rs` | The verb registry. It drives both the CLI and the MCP tools. |
| `crates/core/` | The shared core: contract, wire client, state, process control, and the iOS host adapter. |
| `crates/android/` | The Android host side: ADB discovery, boot, install, token, forward, the loopback bridge, `AndroidAdapter`, and `AndroidSession`. |
| `drivers/ios/` | The iOS driver: an Xcode project. Its UI-test target hosts the HTTP server (`Driver/AgentMobileServer.swift`). `Host/` is the small app the runner attaches to. `am.sh` is a curl helper. `tunnel-cloudflared.sh` is the example tunnel. |
| `drivers/android/` | The Android driver: a Kotlin `AccessibilityService`, a `ProvisionProvider` that makes tokens, and the Gradle project (`gradlew`). |
| `skills/agent-mobile/SKILL.md` | The agent guide that `agent-mobile skills` prints. |
| `npm/` | The macOS package. Its `bin` is the native binary, with no JS shim and no install script. `scripts/sync-npm-version.sh` makes `bin/`, `runner/`, and `android/`. |
| `scripts/` | `setup-android-sdk.sh` (SDK install and check), `ci-boot-android-emulator.sh` (CI emulator), `record-fixtures.sh` (fixture capture), `check-*.sh` (source rules), `bench.py`. |
| `docs/ARCHITECTURE.md` | How it works: the wire protocol, the MCP server, sessions, and security. |

- Each verb parses its arguments, builds JSON, and calls the same wire.
- Only discovery, `serve`, and lazy start do platform lifecycle work.
- There is no `crates/ios`. The iOS host adapter stays in `crates/core`.

## Checks

Run these before you open a PR. CI runs the same ones.

```
python3 -m unittest discover -s scripts -p '*_test.py'   # script self-tests
scripts/check-rust-source.sh                             # file size and test rules
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
git diff --check
```

## Rust rules

Each rule fails the build. Together they are the Rust version of an "anti-slop" ruleset.

| Rule | Enforced by |
|---|---|
| Format with `cargo fmt`. | `cargo fmt --all -- --check` |
| Clippy with the pedantic group, warnings as errors. Never `dbg!`, `todo!`, `unimplemented!`, `panic!`, `unwrap`, `expect`, `unsafe`, or `#[allow]` without a reason. | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| At most 12 cognitive complexity and 100 lines per function. | `clippy.toml` |
| At most 400 lines per `.rs` file. | `scripts/check-rust-source.sh` |
| No inline comments (`//`, `/* */`). Only `///` and `//!` doc comments, at most 15 lines each. | `scripts/check_rust_comments.py` |
| Each public item has a doc comment. | `missing_docs` |
| Each `#[test]` asserts something. No `sleep` in a test. `#[ignore]` has a reason. | `scripts/check-rust-source.sh` |
| Advisories, licenses, bans, and sources pass. | `cargo deny check` |

Core tests compare against fixtures recorded from real driver output, so a test never copies the code it checks.

## Tests

| Kind | Proves | Needs a live target |
|---|---|---|
| Unit tests | the text formatter and ref parsing | no |
| Golden fixture tests (in-crate) | the core parses real iOS and Android driver JSON correctly | no |
| Platform integration tests | the public CLI starts, snapshots, and cleans a real platform session | yes |

Fixtures come from live drivers, never by hand. So the core can be tested without a device.

- `tests/integration_snapshot.rs` covers iOS.
- `tests/integration_android.rs` covers parallel lazy start, real Android verbs, screenshot decoding, exact cleanup of the forward, and a second serve.

## CI

Three jobs. Physical devices are never in CI.

| Job | Runner | Steps | Gate |
|---|---|---|---|
| `lint-and-test` | `ubuntu-latest` | format, clippy, source rules, workspace tests, dependency policy | any failed step blocks the merge |
| `simulator-integration` | `macos-26` | build the iOS driver; run the ignored `integration_snapshot` | the live snapshot assertion must pass |
| `android-emulator` | `ubuntu-latest` | install only the missing pinned headless SDK parts; build, lint, and test the APK; boot the disposable API 37 AVD; run the ignored `integration_android`; always clean what it owns | the full public-CLI Android path must pass |

Workflow hygiene: `permissions: {}` at the top, `contents: read` per job, SHA-pinned actions, a `timeout-minutes` per job, and `--locked` on each cargo command.

## Android driver

You need only the command-line SDK, not Android Studio. Install it with `scripts/setup-android-sdk.sh` (`--check` checks without an install). Build and test the Kotlin driver with its Gradle wrapper:

```
drivers/android/gradlew -p drivers/android :app:testDebugUnitTest :app:lintDebug :app:assembleDebug --no-daemon
```

## Live Android test

This test is `#[ignore]`d. It needs a booted, authorized emulator and a built driver APK:

```
ANDROID_SDK_ROOT=/path/to/android-sdk ANDROID_AVD_HOME=/path/to/android-avd \
    AGENT_MOBILE_ANDROID_APK=drivers/android/app/build/outputs/apk/debug/app-debug.apk \
    cargo test --test integration_android --locked -- --ignored --nocapture
```

It starts and cleans two sessions through the public CLI. It keeps any `adb forward` rows that were there before, the emulator, the installed APK, and the enabled service.

## Record fixtures

Record golden fixtures from a real driver. Never write them by hand.

```
AGENT_MOBILE_URL=<driver-url> AGENT_MOBILE_TOKEN_FILE=<0600 token file> \
    scripts/record-fixtures.sh ios       # or: android
```

- `AGENT_MOBILE_TOKEN_FILE` points at the session token file, so the token never shows in argv or output.
- Android fixtures need a live full bridge (`serve android:<target>`).
- When you record again, update `crates/core/tests/fixtures/PROVENANCE.md` with the source commit, the device or AVD, the OS, and the tool versions.

## Benchmarks

```sh
cargo build --release --locked
python3 scripts/bench.py                              # about 25 min: 3 cold boots, 30 iterations, soak, scenario
python3 scripts/bench.py --quick                      # about 5 min smoke run
python3 scripts/bench.py --device android:avd:<name>  # same suite on Android
python3 scripts/bench.py --compare bench-results/<old>.json
python3 scripts/bench.py --render bench-results/<run>.json  # rebuild the HTML only
```

- `scripts/bench_agent_loop.py` drives the MCP server through a fixed Settings session on the iOS simulator and the Android emulator. It prints warm-up time, time per action, and gates as JSON.
- `scripts/bench_install.py <tarball>` installs a packed `npm/` with npm, pnpm, bun, and yarn into temp folders. It prints start-up times and install gates as JSON.

## Release and versioning

- Use a Conventional Commit title for each change. A breaking change to the wire protocol uses `feat!:` or a `BREAKING CHANGE:` footer.
- The npm package `bin` is the native binary itself. It has no JS shim and no install script, so each package manager runs it directly.
- `preferUnplugged` stops Yarn Plug'n'Play from keeping the binary inside a zip archive.
- The package ships `runner/` (the iOS runner) and `android/` (the Android driver APK). An installed CLI drives both platforms with no checkout, no Xcode build, and no Gradle run.
- `crates/core` stays `publish = false`.
- The protocol version is separate from the crate version. A breaking protocol change bumps the envelope version, and the driver and the CLI ship together.
