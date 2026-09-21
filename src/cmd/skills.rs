//! `skills`: the one-page agent guide, written from the shipped surface and
//! kept honest by the two-way accuracy test.

/// The guide text; every command named here exists in `--help`, and the test
/// proves it in both directions.
const GUIDE: &str = "\
agent-mobile — drive an iOS app through a snapshot -> act loop

COMMANDS
  devices                        list reachable simulators and paired devices
  status                         active app, device, os, current snapshot id
  snapshot                       mint refs and print the accessibility tree
  tap <ref> | <x> <y>            tap an element ref, or a point in the app frame
  type [<ref>] <text...>         type text; a leading ref taps that field first
  swipe <up|down|left|right> [<ref>]
                                 swipe the app, or one element
  home                           press Home; returns the springboard tree
  launch <bundle_id>             cold-start the app; kills saved state
  activate <bundle_id>           resume a running app; keeps its state
  screenshot [path]              PNG to a file, or base64 to stdout
  stop                           terminate the active app; the driver stays up
  skills                         this page

FLAGS (global)
  --json                         emit the raw JSON envelope
  --app <bundle>                 retarget `snapshot` at a bundle id
  --max-depth <n>                trim the tree client-side; header then shows
                                 complete=false
  --device <name>                pick a device; remembered for later calls

ENVIRONMENT
  AGENT_MOBILE_URL, AGENT_MOBILE_TOKEN   override the saved session per call

THE LOOP
  snapshot -> pick a ref -> act -> repeat. Every action replies with the next
  snapshot, so tap, type, and swipe already hand you fresh refs.

REFS
  Refs look like @<snapshot>:e<N> and die with their snapshot. After any
  action, use refs from the newest snapshot only.
  STALE_REF          -> re-snapshot, then retry with a fresh ref
  AMBIGUOUS_TARGET   -> pick a ref with a firmer native_id, walk bounds via
                        --json, or coordinate-tap with `tap <x> <y>`

OUTPUT CONTRACT
  stdout carries parseable data; stderr carries hints and errors.
  settled=false means the settle loop hit its cap; the tree is still usable.
  complete=false means --max-depth trimmed nodes below the printed depth.
  Exit codes: 0 ok, 1 driver or transport failure, 2 usage error.

LAUNCH VS ACTIVATE
  launch cold-starts and destroys saved state; activate resumes what is
  already running. Default to activate; use launch only for a clean start.
";

/// Run `skills`; prints the guide, no wire involved.
pub fn run() -> i32 {
    print!("{GUIDE}");
    0
}
