//! Lazy start (KTD7): a verb with no session takes the atomic boot lock,
//! spawns `serve` detached, and waits for the state entry it writes. A
//! second caller waits on the same lock instead of racing the boot.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Command, Stdio};
use std::time::Instant;

use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::process::{BOOT_BUDGET, BOOT_POLL, BootLock};

use super::{Ctx, Session, ensure_state_dir};

/// Resolve a session, booting a driver first when none exists.
pub fn session(ctx: &Ctx) -> Result<Session, Failure> {
    ensure_state_dir(&ctx.store)?;
    let lock_path = ctx.store.lock_file();
    let deadline = Instant::now() + BOOT_BUDGET;
    loop {
        if Instant::now() > deadline {
            return Err(Failure::local(
                "timed out waiting for a driver to come up",
                "run `agent-mobile serve <device>` yourself to see the boot log",
            ));
        }
        if let Some(lock) = BootLock::take(&lock_path)? {
            return boot_with_lock(ctx, lock, deadline);
        }
        if BootLock::age(&lock_path).is_some_and(|a| a > BOOT_BUDGET) {
            BootLock::clear(&lock_path);
            continue;
        }
        if let Some(s) = ctx.ready_session()? {
            return Ok(s);
        }
        std::thread::sleep(BOOT_POLL);
    }
}

/// We hold the boot lock: re-check state, pick a device, spawn `serve`
/// detached with output to the driver log, then wait for its entry.
fn boot_with_lock(ctx: &Ctx, _lock: BootLock, deadline: Instant) -> Result<Session, Failure> {
    if let Some(s) = ctx.ready_session()? {
        return Ok(s);
    }
    let device = pick_device(ctx)?;
    eprintln!("no driver running; starting one for {device}…");
    let log = ctx.store.driver_log(&device);
    let exe = std::env::current_exe()?;
    let out = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)?;
    let err = out.try_clone()?;
    let mut child = Command::new(exe)
        .args(["serve", &device])
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| Failure::local(format!("cannot spawn `serve`: {e}"), "fix and retry"))?;
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::local(
                "the driver did not come up inside the boot budget",
                format!("check the log at {} and retry", log.display()),
            ));
        }
        if let Some(s) = ctx.ready_session()? {
            return Ok(s);
        }
        if let Some(_status) = child.try_wait()? {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            let failure = if super::serve::TRUST_MARKERS.iter().any(|m| text.contains(m)) {
                "the development certificate is not trusted on the device".to_owned()
            } else {
                last_error_line(&text)
            };
            return Err(Failure::local(
                failure,
                format!("check the log at {} and retry", log.display()),
            ));
        }
        std::thread::sleep(BOOT_POLL);
    }
}

/// The last `error:` line in the log — the serve's own diagnosis — or a
/// generic binding failure when the runner died silently.
fn last_error_line(log_text: &str) -> String {
    log_text
        .lines()
        .rev()
        .find(|l| l.starts_with("error: "))
        .map_or_else(
            || "the driver exited before binding".to_owned(),
            |l| l.trim_start_matches("error: ").to_owned(),
        )
}

/// Which device a lazy boot serves: `--device` wins, then the remembered
/// default, then the first iPhone-shaped simulator, then anything.
fn pick_device(ctx: &Ctx) -> Result<String, Failure> {
    if let Some(d) = &ctx.device {
        return Ok(d.clone());
    }
    if let Some(d) = ctx.store.load().default_device {
        return Ok(d);
    }
    let devices = ios::list_devices()?;
    let pick = devices
        .iter()
        .find(|d| d.kind == "simulator" && d.name.contains("iPhone"))
        .or_else(|| devices.iter().find(|d| d.kind == "simulator"))
        .or(devices.first());
    pick.map(|d| d.name.clone()).ok_or_else(|| {
        Failure::local(
            "no devices found",
            "create a simulator with `xcrun simctl create <name> <type>` or pair a device",
        )
    })
}
