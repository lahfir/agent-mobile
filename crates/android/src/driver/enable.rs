//! Accessibility-service enable/bind: settings reads and the
//! emulator-only (exact `ro.kernel.qemu=1`) merge/write path.

use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;

use crate::adb::Adb;
use crate::driver::{BIND_BUDGET, PACKAGE, SERVICE_COMPONENT};

/// Merge our service component into a `settings secure` value, preserving
/// existing colon-separated entries in order and never duplicating.
#[must_use]
pub(crate) fn merge_enabled_services(raw: &str, ours: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for piece in raw.split(':').map(str::trim) {
        if piece.is_empty() || piece == "null" {
            continue;
        }
        if !parts.iter().any(|p| p == piece) {
            parts.push(piece.to_owned());
        }
    }
    let present = parts.iter().any(|p| p == ours)
        || (ours == SERVICE_COMPONENT && parts.iter().any(|p| is_our_service_entry(p)));
    if !present {
        parts.push(ours.to_owned());
    }
    parts.join(":")
}

/// Is this colon-list entry exactly our service, full or short form?
fn is_our_service_entry(entry: &str) -> bool {
    let Some((pkg, class)) = entry.trim().split_once('/') else {
        return false;
    };
    pkg == PACKAGE
        && (entry.trim() == SERVICE_COMPONENT || class == ".AgentMobileAccessibilityService")
}

/// Bounded `settings get/put` helpers.
fn settings_get(adb: &Adb, serial: &str, key: &str) -> Result<String, Failure> {
    let out = adb.remote_shell_ok(serial, "settings read", &["settings", "get", "secure", key])?;
    Ok(out.stdout.trim().to_owned())
}

fn settings_put(adb: &Adb, serial: &str, key: &str, value: &str) -> Result<(), Failure> {
    adb.remote_shell_ok(
        serial,
        "settings write",
        &["settings", "put", "secure", key, value],
    )
    .map(|_| ())
    .map_err(|_| bind_failure("settings write was refused"))
}

/// Enable and verify the accessibility service: writes happen only when
/// the merged value differs, then a bounded settle loop re-reads and
/// conditionally re-writes until the exact entry holds, and a bounded
/// `dumpsys` poll confirms the service actually bound.
///
/// # Errors
/// [`Failure::Local`] when writes fail, the service does not appear in
/// `enabled_accessibility_services`, or it never binds; the refusal names
/// Settings > Accessibility and App Info > Allow restricted settings.
pub(crate) fn enable_service(adb: &Adb, serial: &str) -> Result<(), Failure> {
    enable_service_bounded(adb, serial, BIND_BUDGET)
}

/// [`enable_service`] with an explicit settle-and-bind deadline.
pub(crate) fn enable_service_bounded(
    adb: &Adb,
    serial: &str,
    budget: Duration,
) -> Result<(), Failure> {
    let raw = settings_get(adb, serial, "enabled_accessibility_services")?;
    let accessibility = settings_get(adb, serial, "accessibility_enabled")?;
    if raw.split(':').any(is_our_service_entry) && accessibility == "1" {
        return verify_service(adb, serial, budget);
    }
    let qemu = adb
        .remote_shell(serial, &["getprop", "ro.kernel.qemu"])
        .is_ok_and(|out| out.success && out.stdout.trim() == "1");
    if !qemu {
        return Err(bind_failure("is not enabled"));
    }
    let deadline = Instant::now() + budget;
    let mut accessibility_done = accessibility == "1";
    loop {
        if adb.is_cancelled() {
            return Err(Failure::local("operation interrupted", "rerun the command"));
        }
        let fresh = settings_get(adb, serial, "enabled_accessibility_services")?;
        let merged = merge_enabled_services(&fresh, SERVICE_COMPONENT);
        if merged != fresh {
            settings_put(adb, serial, "enabled_accessibility_services", &merged)?;
        }
        if !accessibility_done {
            settings_put(adb, serial, "accessibility_enabled", "1")?;
            accessibility_done = true;
        }
        if service_present(adb, serial)? {
            return poll_bound_until(adb, serial, deadline);
        }
        if Instant::now() > deadline {
            return Err(bind_failure("service not present after write"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Reread the setting for an exact component match, then poll `dumpsys
/// accessibility` for the actual bound-service record line.
fn verify_service(adb: &Adb, serial: &str, budget: Duration) -> Result<(), Failure> {
    if !service_present(adb, serial)? {
        return Err(bind_failure("service not present after write"));
    }
    poll_bound_until(adb, serial, Instant::now() + budget)
}

/// Is our exact component in `enabled_accessibility_services` right now?
fn service_present(adb: &Adb, serial: &str) -> Result<bool, Failure> {
    let reread = settings_get(adb, serial, "enabled_accessibility_services")?;
    Ok(reread.split(':').any(is_our_service_entry))
}

/// Poll `dumpsys accessibility` for the bound-service record until
/// `deadline`, honouring cancellation.
fn poll_bound_until(adb: &Adb, serial: &str, deadline: Instant) -> Result<(), Failure> {
    loop {
        if adb.is_cancelled() {
            return Err(Failure::local("operation interrupted", "rerun the command"));
        }
        let out = adb.remote_shell(serial, &["dumpsys", "accessibility"])?;
        if out
            .stdout
            .lines()
            .any(|l| l.contains("Service[label=Agent Mobile Driver,"))
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(bind_failure("service never bound"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The standard refusal with its manual remedy.
fn bind_failure(why: &str) -> Failure {
    Failure::local(
        format!("accessibility service {why}"),
        "enable it in Settings > Accessibility > Agent Mobile Driver; on Android 13+ \
         first allow App Info > Allow restricted settings",
    )
}
