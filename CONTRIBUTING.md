# Contributing

## One-time setup

```
git config core.hooksPath .githooks
```

The hook runs the same gates as CI before every commit that touches Rust.

## Rust rules

All of these fail the build. They are the Rust equivalent of an "anti-slop" ruleset.

- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`
  with the pedantic group on. Denied outright: `dbg!`, `todo!`, `unimplemented!`, `panic!`,
  `unwrap`, `expect`, `unsafe`, and `#[allow]` without a reason.
- Cognitive complexity at most 12 per function, at most 100 lines per function
  (`clippy.toml`).
- At most 400 lines per `.rs` file (`scripts/check-rust-source.sh`).
- No inline comments. `//` and `/* */` are rejected; only `///` and `//!` doc comments are
  allowed, and a doc comment is at most 15 lines (`scripts/check_rust_comments.py`).
- Public items carry a doc comment (`missing_docs`).
- Test rules, same script: every `#[test]` asserts something, no `sleep` in a test, and
  `#[ignore]` carries a reason. Core tests compare against fixtures recorded from real driver
  output, so a test never re-implements the code it checks.
- `cargo deny check` gates advisories, licenses, bans, and sources.

## Deterministic gates

```
python3 scripts/check_rust_comments_test.py   # comment-rule self-tests
scripts/check-rust-source.sh                  # file size + test hygiene
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
git diff --check
```

## Android driver

No Android Studio needed — only the command-line SDK via
`scripts/setup-android-sdk.sh` (use `--check` to verify without installing). The Kotlin driver
project is built and tested with its checked-in Gradle wrapper:

```
drivers/android/gradlew -p drivers/android :app:testDebugUnitTest :app:lintDebug :app:assembleDebug --no-daemon
```

## Recording fixtures

Golden fixtures are captured from a real driver, never hand-written:

```
AGENT_MOBILE_URL=<driver-url> AGENT_MOBILE_TOKEN_FILE=<0600 token file> \
    scripts/record-fixtures.sh ios       # or: android
```

`AGENT_MOBILE_TOKEN_FILE` points at the session token file — the token never appears in argv
or output. Android fixtures require a live full U6 bridge (`serve android:<target>`); when
re-recording, update `crates/core/tests/fixtures/PROVENANCE.md` with the source commit,
device/AVD, OS, and tool versions.

## Commits

Conventional Commit titles. A breaking wire-protocol change uses `feat!:` or a
`BREAKING CHANGE:` footer and bumps the protocol version.
