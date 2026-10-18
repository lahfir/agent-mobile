//! `skills`: the one-page agent guide, projected from the shipped surface.

/// Static sections of the guide; command and flag sections come from the registry.
const GUIDE: &str = "\
agent-mobile — drive an iOS or Android app through a snapshot -> act loop

ENVIRONMENT
  AGENT_MOBILE_URL, AGENT_MOBILE_TOKEN   override the saved session per call;
                                 set BOTH — a URL alone has no token to pair
                                 with and fails as a usage error. Use these
                                 for tunnel URLs (https://…trycloudflare.com).
  ANDROID_HOME, ANDROID_SDK_ROOT SDK location, in that precedence order;
                                 then the platform default, then PATH.
                                 scripts/setup-android-sdk.sh --check
                                 verifies without installing.
  AGENT_MOBILE_BOOT_BUDGET_SECS  lazy-boot deadline in seconds (default
                                 240). A cold Gradle build may run the
                                 full 600 s build bound while lazy boot
                                 kills serve at the deadline — raise past
                                 600 before the first Android boot.

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
  AGENT_MOBILE_REPO_ROOT         checkout root locating drivers/android;
                                 only needed outside a repo checkout
  unauthorized device            a person accepts the USB debugging
                                 prompt, then retry
  Restricted Settings            the \"Agent Mobile Driver\" toggle in
                                 Settings > Accessibility; Android 13+
                                 also needs App Info > ⋮ >
                                 \"Allow restricted settings\" — a person
                                 must flip these on the device
  duplicate device names         select by platform:id key, not the name
  boot timeout                   inspect ~/.agent-mobile/driver-*.log
  physical Android               user-authorized USB/wireless adb plus
                                 Accessibility consent; sideload/debug only
                                 — no Play distribution, no bypass

SYSTEM UI
  Permission dialogs appear in snapshots; tap their buttons by ref like
  any other node. On iOS, the snapshot keeps the target app bundle while
  showing the dialog; after dismissal, use the fresh refs from the action.
  Other system UI lives outside the app under test.
  On iOS that is com.apple.springboard; on Android it is the active
  launcher/system package shown in a snapshot's app field. Act on those
  refs, then return with `snapshot --app <bundle>` on iOS or `back` on
  Android — refs die across apps.

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
  DRIVER_ERROR       -> take a fresh snapshot to verify the current
                        state before retrying; escalate if it recurs
  (client-side codes: USAGE exits 2 before any call; LOCAL/DRIVER_ERROR
   mark failures that never reached the driver)

OUTPUT CONTRACT
  stdout carries parseable data; stderr carries hints and errors.
  settled=false means the settle loop hit its cap; the tree is still usable.
  complete=false means the tree is incomplete — --max-depth trimming or
  driver-side traversal caps, cycles, or unavailable children. Ref actions
  on an incomplete tree fail closed because uniqueness can't be proven.
  Exit codes: 0 ok, 1 driver or transport failure, 2 usage error.

LAUNCH
  launch terminates the process then starts it from the launcher; app
  data is kept. On Android, terminate refuses the driver, System UI, and
  the current launcher. If System UI wedges the foreground, `back` first —
  judge the returned snapshot; `launch` is only a cold-restart fallback
  and loses transient app state.
";

/// The `skills` verb: the guide, no wire involved.
///
/// # Errors
/// Never; the signature matches every local verb.
#[allow(
    clippy::unnecessary_wraps,
    reason = "every local verb shares one fn signature"
)]
pub fn exec(_call: &super::Call) -> Result<super::Reply, agent_mobile_core::error::Failure> {
    let (intro, static_sections) = GUIDE.split_once("ENVIRONMENT\n").unwrap_or((GUIDE, ""));
    let mut guide = intro.to_owned();
    guide.push_str(&crate::surface::usage::guide_sections());
    if !static_sections.is_empty() {
        guide.push_str("ENVIRONMENT\n");
        guide.push_str(static_sections);
    }
    Ok(super::Reply::Text(guide.trim_end_matches('\n').to_owned()))
}
