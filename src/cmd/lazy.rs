//! Lazy start (KTD7): a verb with no session takes the atomic boot lock,
//! spawns `serve` detached, and waits for the state entry it writes. A
//! second caller waits on the same lock instead of racing the boot.

use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::process::{BOOT_POLL, BootLock, boot_budget};

use super::{Ctx, Session, ensure_state_dir};

/// Resolve a session, booting a driver first when none exists.
pub fn session(ctx: &Ctx) -> Result<Session, Failure> {
    ensure_state_dir(&ctx.store)?;
    let lock_path = ctx.store.lock_file();
    let deadline = Instant::now() + boot_budget();
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
        if BootLock::age(&lock_path).is_some_and(|a| a > boot_budget()) {
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
/// detached with output to the driver log, then wait for its entry. A
/// timeout sends TERM first so serve's supervisor can clear state and reap
/// the runner; SIGKILL is only for a serve that refuses.
fn boot_with_lock(ctx: &Ctx, _lock: BootLock, deadline: Instant) -> Result<Session, Failure> {
    if let Some(s) = ctx.ready_session()? {
        return Ok(s);
    }
    let device = pick_device(ctx)?;
    eprintln!("no driver running; starting one for {device}…");
    let log = ctx.store.driver_log(&device);
    let log_base = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    let exe = std::env::current_exe()?;
    let out = agent_mobile_core::process::open_private_log(&log)?;
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
            let _ = agent_mobile_core::process::terminate(child.id());
            if !agent_mobile_core::process::await_exit(child.id(), Duration::from_secs(5)) {
                let _ = child.kill();
            }
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
            let text = std::fs::File::open(&log)
                .and_then(|mut f| {
                    f.seek(SeekFrom::Start(log_base))?;
                    let mut s = String::new();
                    f.read_to_string(&mut s).map(|_| s)
                })
                .unwrap_or_default();
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
/// default, then the first iPhone-shaped simulator, then any simulator,
/// then a physical device — `devicectl` only runs when no simulator exists.
/// A remembered UDID or stale name is healed to the canonical name here —
/// `serve` keys the session under it and the waiter polls `resolve` through
/// the same `default_device`, so the two must agree.
fn pick_device(ctx: &Ctx) -> Result<String, Failure> {
    if let Some(d) = &ctx.device {
        return Ok(d.clone());
    }
    if let Some(d) = ctx.store.load().default_device {
        if let Ok(Some(found)) = ios::find_device(&d) {
            if found.name != d {
                let _ = ctx.store.remember_device(&found.name);
            }
            return Ok(found.name);
        }
        return Ok(d);
    }
    let sims = ios::simulators()?;
    let pick = sims
        .iter()
        .find(|d| d.name.contains("iPhone"))
        .or_else(|| sims.first())
        .cloned()
        .or_else(|| ios::physical().into_iter().next());
    pick.map(|d| d.name).ok_or_else(|| {
        Failure::local(
            "no devices found",
            "create a simulator with `xcrun simctl create <name> <type>` or pair a device",
        )
    })
}
