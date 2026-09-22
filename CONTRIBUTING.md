# Contributing

## One-time setup

```
git config core.hooksPath .githooks
```

The hook runs the same gates as CI before every commit that touches Rust.

## Rust rules

All of these fail the build. They are the Rust equivalent of an "anti-slop" ruleset.

- `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` with the pedantic
  group on. Denied outright: `dbg!`, `todo!`, `unimplemented!`, `panic!`, `unwrap`, `expect`,
  `unsafe`, and `#[allow]` without a reason.
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

## Commits

Conventional Commit titles. A breaking wire-protocol change uses `feat!:` or a
`BREAKING CHANGE:` footer and bumps the protocol version.
