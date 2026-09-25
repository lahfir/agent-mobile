//! `skills`: the one-page agent guide, written from the shipped surface and
//! kept honest by the two-way accuracy test.

/// The guide text; every command named here exists in `--help`, and the test
/// proves it in both directions.
const GUIDE: &str = "\
agent-mobile — drive an iOS app through a snapshot -> act loop

COMMANDS
  devices                        list reachable simulators and paired devices
  serve <device> [--app <id>]    start a driver in the foreground; --app
                                 launches the bundle once the driver binds.
                                 Prints url/device/token to a terminal; when
                                 piped the token lives only in
                                 ~/.agent-mobile/tokens/<device>
  status                         active app, device, os, current snapshot id
  snapshot [--app <id>]          mint refs and print the accessibility tree;
                                 --app retargets a different bundle
  tap <ref> | <x> <y>            tap an element ref, or a point in the app
                                 frame's top-left space (at=x,y is the node's
                                 origin — tap its center x+w/2, y+h/2)
  type [<ref>] <text...>         append text; a leading ref writes that field
                                 directly (the field may not keep focus), else
                                 keys go to the focused field. A newline
                                 presses Return.
                                 Text starting with `-` needs `--`:
                                 type -- -flag
  swipe <up|down|left|right> [<ref>]
                                 swipe the app, or one element
  home                           press Home; returns the springboard tree
  doubletap <ref> | <x> <y>      tap twice on a ref, or a point in the app
                                 frame's top-left space
  pinch <ref> <scale> [--velocity <v>]
                                 zoom on a ref; above 1 zooms out, below 1
                                 zooms in. Near-1 or non-positive scales
                                 fail as BAD_REQUEST; non-finite
                                 scale/velocity fail here
  hold <ref> | <x> <y> [--duration <s>]
                                 press and hold 0 < d <= 10 s, default 1.0;
                                 native context menus surface in the
                                 snapshot (web long-press is unproven)
  back                           system edge swipe back; no target. Judge
                                 from the returned tree: web-history
                                 back is unproven
  twofinger <ref>                two-finger tap on a ref
  center notification            open Notification Center (SpringBoard
                                 session). Earlier refs die; snapshot or
                                 --app to return to your app
  launch <bundle_id>             cold-start the app; kills saved state
  screenshot [path]              PNG to a file, or base64 to stdout
                                 (--json stays base64; path+--json is an error)
  stop                           terminate the active app; the driver stays
                                 up. No tree comes back — snapshot to verify
  skills                         this page

FLAGS (global)
  --json                         emit the raw JSON envelope; failures then
                                 print the envelope shape on stdout too
  --app <bundle>                 retarget `snapshot`/`serve`
  --max-depth <n>                trim the tree client-side; header then shows
                                 complete=false
  --device <name>                pick a device by name or UDID; remembered
                                 for later calls

ENVIRONMENT
  AGENT_MOBILE_URL, AGENT_MOBILE_TOKEN   override the saved session per call;
                                 set BOTH — a URL alone has no token to pair
                                 with and fails as a usage error. Use these
                                 for tunnel URLs (https://…trycloudflare.com).

SESSIONS
  Any verb starts the driver on demand when none runs — the first call can
  take a minute while the runner builds and the simulator boots. `serve`
  runs the driver in the foreground instead and prints the token once.
  One driver serves one device on port 8770; serving a second device means
  stopping the first serve.

SYSTEM DIALOGS
  Permission alerts and other system UI live in com.apple.springboard, not
  the app under test: `snapshot --app com.apple.springboard`, act on its
  refs, then `snapshot --app <your-bundle>` to return.

THE LOOP
  snapshot -> pick a ref -> act -> repeat. Every action replies with the next
  snapshot, so every verb already hands you fresh refs.

REFS AND ERRORS
  Refs look like @<snapshot>:e<N> and die with their snapshot. After any
  action, use refs from the newest snapshot only.
  STALE_REF          -> re-snapshot, then retry with a fresh ref
  AMBIGUOUS_TARGET   -> pick a ref with a firmer native_id, walk bounds via
                        --json, or coordinate-tap with `tap <x> <y>`
  BAD_REQUEST        -> fix the request; do not retry unchanged
  UNKNOWN_COMMAND    -> fix the client; do not retry
  UNAUTHORIZED       -> fix AGENT_MOBILE_TOKEN; do not retry unchanged
  DRIVER_ERROR       -> retry once; escalate if it recurs
  (client-side codes: USAGE exits 2 before any call; LOCAL/DRIVER_ERROR
   mark failures that never reached the driver)

OUTPUT CONTRACT
  stdout carries parseable data; stderr carries hints and errors.
  settled=false means the settle loop hit its cap; the tree is still usable.
  complete=false means --max-depth trimmed nodes below the printed depth.
  Exit codes: 0 ok, 1 driver or transport failure, 2 usage error.

LAUNCH
  launch cold-starts and destroys saved state; use it for a clean start.
";

/// Run `skills`; prints the guide, no wire involved.
pub fn run() -> i32 {
    super::emit(GUIDE.trim_end_matches('\n'));
    0
}
