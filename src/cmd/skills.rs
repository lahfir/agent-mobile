//! `skills`: the one-page agent guide, written from the shipped surface and
//! kept honest by the two-way accuracy test.

/// The guide text; every command named here exists in `--help`, and the test
/// proves it in both directions.
const GUIDE: &str = "\
agent-mobile — drive an iOS or Android app through a snapshot -> act loop

COMMANDS
  devices                        list iOS simulators/phones and Android
                                 AVD/USB/wireless targets
  serve <device> [--app <id>]    start a driver in the foreground; <device>
                                 is a name, id, or collision-free
                                 platform:id key (`ios:<udid>`,
                                 `android:avd:<name>`, `android:<serial>`).
                                 --app launches the app once the driver
                                 binds. Prints url/device/token to a
                                 terminal; when piped the token lives only
                                 in ~/.agent-mobile/tokens/<device>
  status                         active app, device, os, current snapshot id
  snapshot [--app <id>]          mint refs and print the accessibility tree;
                                 --app carries an app selector: iOS targets
                                 the bundle, Android only validates the
                                 foreground package — use launch to change
                                 apps
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
  home                           go to the launcher/home; returns that tree
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
  back                           navigate back; no target. Judge from the
                                 returned tree: web-history back is unproven
  twofinger <ref>                two-finger tap on a ref
  center notification            open notifications (system UI session).
                                 Earlier refs die; snapshot or --app to
                                 return to your app
  launch <app_id>                cold-start or restart an app (iOS bundle id
                                 or Android package); app data is kept
  screenshot [path]              PNG to a file, or base64 to stdout
                                 (--json stays base64; path+--json is an error)
  stop                           terminate the active app; the driver stays
                                 up. No tree comes back — snapshot to verify
  skills                         this page

FLAGS (global)
  --json                         emit the raw JSON envelope; failures then
                                 print the envelope shape on stdout too
  --app <app_id>                 app selector for `snapshot`/`serve`; on
                                 Android `snapshot --app` only checks the
                                 already-foreground package
  --max-depth <n>                trim the tree client-side; header then shows
                                 complete=false
  --device <device>              pick a device by name, id, or platform:id;
                                 remembered for later calls

ENVIRONMENT
  AGENT_MOBILE_URL, AGENT_MOBILE_TOKEN   override the saved session per call;
                                 set BOTH — a URL alone has no token to pair
                                 with and fails as a usage error. Use these
                                 for tunnel URLs (https://…trycloudflare.com).

SESSIONS
  Any verb starts the driver on demand when none runs — the first call can
  take a minute while a runner builds or a device boots. `serve` runs one
  long-lived driver in the foreground and prints the token once. iOS owns a
  runner process on fixed port 8770; Android owns a localhost bridge plus
  one ephemeral `adb forward`. Stopping an Android serve removes only its
  bridge/forward/state/token — the emulator stays up, the APK stays
  installed, the accessibility service stays enabled.

ANDROID SETUP / RECOVERY
  scripts/setup-android-sdk.sh --check
                                 verifies the SDK without installing; the
                                 bare command installs only missing pinned
                                 components (idempotent). Does not install
                                 or require Android Studio
  AGENT_MOBILE_ANDROID_APK       path override for the driver APK; otherwise
                                 the checked-in Gradle wrapper builds it
  unauthorized device            accept the USB debugging prompt, then retry
  Restricted Settings            the \"Agent Mobile Driver\" toggle in
                                 Settings > Accessibility; Android 13+
                                 also needs App Info > ⋮ >
                                 \"Allow restricted settings\"
  duplicate device names         select by platform:id key, not the name
  boot timeout                   inspect ~/.agent-mobile/driver-*.log
  physical Android               user-authorized USB/wireless adb plus
                                 Accessibility consent; sideload/debug only
                                 — no Play distribution, no bypass

SYSTEM UI
  Permission alerts and other system UI live outside the app under test.
  On iOS that is com.apple.springboard; on Android it is the active
  launcher/system package shown in a snapshot's app field. Act on those
  refs, then return with `snapshot --app <bundle>` on iOS or
  `launch <package>` on Android — refs die across apps.

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
  launch terminates the process then starts it from the launcher; app
  data is kept. On Android, terminate refuses the driver, System UI, and
  the current launcher.
";

/// Run `skills`; prints the guide, no wire involved.
pub fn run() -> i32 {
    super::emit(GUIDE.trim_end_matches('\n'));
    0
}
