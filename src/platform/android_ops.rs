//! Android-only runtime helpers kept pure so tests need no `adb`: serial
//! resolution, state-entry metadata, and stale-row cleanup.

use std::path::Path;
use std::time::Duration;

use agent_mobile_android::{AndroidAdapter, AndroidDeviceState};
use agent_mobile_core::error::Failure;
use agent_mobile_core::state::SessionEntry;

/// Resolve the `adb` serial to start a session on: an AVD target — running
/// or shutdown — always goes through `boot` so `sys.boot_completed` is
/// proven even on a correlated row; a non-AVD target uses its own serial
/// after the offline/unauthorized gate. Returns `(serial, emulator_pid)`.
pub(crate) fn android_serial(
    target: &agent_mobile_android::AndroidTarget,
    boot: impl FnOnce(&str) -> Result<agent_mobile_android::BootedAvd, Failure>,
) -> Result<(String, Option<u32>), Failure> {
    if let Some(avd) = target.avd.as_deref() {
        let booted = boot(avd)?;
        return Ok((booted.serial, (booted.pid > 0).then_some(booted.pid)));
    }
    let Some(s) = &target.serial else {
        return Err(Failure::local(
            format!("{} has no running serial", target.id),
            "start the device or pick another from `agent-mobile devices`",
        ));
    };
    match target.state {
        AndroidDeviceState::Offline => Err(Failure::local(
            format!("{s} is offline"),
            "run `adb reconnect` or replug the device and retry",
        )),
        AndroidDeviceState::Unauthorized => Err(Failure::local(
            format!("{s} is unauthorized"),
            "accept the USB debugging prompt on the device and retry",
        )),
        _ => Ok((s.clone(), None)),
    }
}

/// Fill `e` with the Android cleanup metadata — the pure half of
/// `session_entry`, kept separate so tests can verify it without a live
/// session.
pub(crate) fn android_entry_fields(
    e: &mut SessionEntry,
    serial: Option<&str>,
    forward_port: Option<u16>,
    bridge_port: Option<u16>,
    apk_source: Option<&Path>,
    emulator_pid: Option<u32>,
    log: &Path,
) {
    e.serial = serial.map(str::to_owned);
    e.forward_port = forward_port;
    e.bridge_port = bridge_port;
    e.apk_source = apk_source.map(|p| p.to_string_lossy().into_owned());
    e.emulator_pid = emulator_pid;
    e.log_file = Some(log.to_string_lossy().into_owned());
}

/// Reclaim a dead serve's leftovers for its recorded platform: a P1/iOS
/// row reaps the recorded xcodebuild runner (TERM + bounded wait); an
/// `android` row removes exactly the recorded `serial`/`forward_port` —
/// never another row, never the emulator, service, or APK. Incomplete
/// Android metadata is an actionable failure rather than a guess.
///
/// # Errors
/// [`Failure::Local`] when required metadata is missing or the cleanup
/// command fails.
pub fn cleanup_stale(entry: &SessionEntry) -> Result<(), Failure> {
    if entry.platform.as_deref() == Some("android") {
        let (Some(serial), Some(port)) = (entry.serial.as_deref(), entry.forward_port) else {
            return Err(Failure::local(
                "stale Android session entry lacks serial/forward_port metadata",
                "inspect `adb forward --list`, then remove the session row with \
                 `adb -s <serial> forward --remove tcp:<port>`",
            ));
        };
        return AndroidAdapter::from_environment()?.remove_owned_forward(serial, port);
    }
    if let Some(p) = entry.platform.as_deref().filter(|p| *p != "ios") {
        return Err(Failure::local(
            format!("session entry has unrecognized platform {p:?}"),
            "inspect the row in ~/.agent-mobile/state.json and fix or remove it",
        ));
    }
    if let Some(rpid) = entry.runner_pid
        && agent_mobile_core::process::pid_alive(rpid)
        && agent_mobile_core::process::terminate_runner(rpid)
    {
        let _ = agent_mobile_core::process::await_exit(rpid, Duration::from_secs(5));
    }
    Ok(())
}
