//! Lazy start (KTD7): a verb with no session takes the atomic boot lock,
//! spawns `serve` detached, and waits for the state entry it writes. A
//! second caller waits on the same lock instead of racing the boot.

use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::process::{BOOT_POLL, BootLock, boot_budget};

use super::{Ctx, Session, ensure_state_dir};

/// Resolve a session, booting a driver first when none exists.
pub fn session(ctx: &Ctx) -> Result<Session, Failure> {
    ensure_state_dir(&ctx.store)?;
    super::served_elsewhere(&ctx.store.load(), ctx.device.as_deref())?;
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
        if let Some(s) = ctx.ready_session(ctx.device.as_deref())? {
            return Ok(s);
        }
        std::thread::sleep(BOOT_POLL);
    }
}

/// Grace period for a timed-out `serve` to run its own cleanup (several
/// bounded Android ADB teardowns) before we force-kill it.
const SERVE_CLEANUP_BUDGET: Duration = Duration::from_secs(60);

/// We hold the boot lock: re-check state, pick a device, spawn `serve`
/// detached with output to the driver log, then wait for its entry. A
/// timeout sends TERM first so serve's supervisor can clear state and reap
/// the runner; SIGKILL is only for a serve that refuses.
fn boot_with_lock(ctx: &Ctx, _lock: BootLock, deadline: Instant) -> Result<Session, Failure> {
    if let Some(s) = ctx.ready_session(ctx.device.as_deref())? {
        return Ok(s);
    }
    let device = pick_device(ctx)?;
    super::served_elsewhere(&ctx.store.load(), Some(&device))?;
    eprintln!("no driver running; starting one for {device}…");
    if physical_ios(&device) {
        eprintln!("keep the iPhone unlocked until the driver answers");
    }
    let log = ctx.store.driver_log(&device);
    let log_base = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    let exe = std::env::current_exe()?;
    let out = agent_mobile_core::process::open_private_log(&log)?;
    let err = out.try_clone()?;
    let mut command = Command::new(exe);
    if !ctx.remember {
        command.env(super::serve::NO_REMEMBER_ENV, "1");
    }
    let mut child = command
        .args(["serve", &device])
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| Failure::local(format!("cannot spawn `serve`: {e}"), "fix and retry"))?;
    loop {
        if Instant::now() > deadline {
            let _ = agent_mobile_core::process::terminate(child.id());
            let stop = Instant::now() + SERVE_CLEANUP_BUDGET;
            while child.try_wait()?.is_none() && Instant::now() < stop {
                std::thread::sleep(Duration::from_millis(100));
            }
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
            return Err(Failure::local(
                "the driver did not come up inside the boot budget",
                format!("check the log at {} and retry", log.display()),
            ));
        }
        if let Some(s) = ctx.ready_session(Some(&device))? {
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
            if let Some(failure) = crate::platform::refusal(&text) {
                return Err(failure);
            }
            let (cause, next) = relay_diagnosis(&text);
            return Err(Failure::local(
                cause,
                next.unwrap_or_else(|| format!("check the log at {} and retry", log.display())),
            ));
        }
        std::thread::sleep(BOOT_POLL);
    }
}

/// A physical iPhone key: simulator UDIDs are 36-char UUIDs, device UDIDs are not.
fn physical_ios(key: &str) -> bool {
    key.strip_prefix("ios:").is_some_and(|id| id.len() != 36)
}

fn relay_diagnosis(log_text: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = log_text.lines().collect();
    let Some(index) = lines.iter().rposition(|line| line.starts_with("error: ")) else {
        return ("the driver exited before binding".to_owned(), None);
    };
    let cause = lines[index].trim_start_matches("error: ").to_owned();
    let next = lines
        .get(index + 1)
        .filter(|line| line.starts_with("next: "))
        .map(|line| line.trim_start_matches("next: ").to_owned());
    (cause, next)
}

/// Which device a lazy boot serves: `--device` wins, then the remembered
/// default healed to a collision-free key, then the platform default —
/// iPhone-named simulator, any iOS simulator, iOS physical, ready Android
/// target, any Android target. `serve` keys the session under the same
/// key the waiter polls through `default_device`, so the two must agree.
fn pick_device(ctx: &Ctx) -> Result<String, Failure> {
    if let Some(d) = &ctx.device {
        return Ok(d.clone());
    }
    pick_device_with(ctx, crate::platform::discover)
}

/// Lazy device selection with the discovery seam injected: live state
/// aliases answer with zero discovery; stored selectors share exactly one
/// scan; a total miss returns the first selector's contextualized error.
pub(super) fn pick_device_with(
    ctx: &Ctx,
    scan: impl FnOnce() -> Result<crate::platform::PlatformScan, Failure>,
) -> Result<String, Failure> {
    let stored = ctx.store.load();
    let selectors: Vec<&str> = stored.default_device_selectors().collect();
    for d in &selectors {
        if let Some(key) = stored.resolve_live_device_key(d)? {
            let display = stored
                .devices
                .get(&key)
                .and_then(|e| e.device_name.clone())
                .unwrap_or_else(|| (*d).to_owned());
            if ctx.remember {
                let _ = ctx.store.remember_device_selection(&key, &display);
            }
            return Ok(key);
        }
    }
    let scan = scan()?;
    let mut first_err = None;
    for d in &selectors {
        match crate::platform::resolve_from(&scan, d) {
            Ok(found) => {
                let key = found.key();
                if ctx.remember {
                    let _ = ctx.store.remember_device_selection(&key, found.name());
                }
                return Ok(key);
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    crate::platform::default_device(&scan).map(|d| d.key()).ok_or_else(|| {
        if scan.notes.is_empty() {
            Failure::local(
                "no devices found",
                "create a simulator with `xcrun simctl create <name> <type>`, pair a device, or create an Android AVD",
            )
        } else {
            Failure::local(
                format!("no devices found; {}", scan.notes.join("; ")),
                "resolve the reported device-discovery failures, then retry",
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{physical_ios, relay_diagnosis};

    #[test]
    fn unlock_hint_is_for_physical_iphones_only() {
        assert!(physical_ios("ios:00008120-001A14C60CE8C01E"));
        assert!(!physical_ios("ios:9B7CEE9E-DED1-4A84-8ABD-E4A381E4CFDD"));
        assert!(!physical_ios("android:avd:agent-mobile-api37"));
    }

    #[test]
    fn relays_serve_next_line() {
        assert_eq!(
            relay_diagnosis("error: phone setup failed\nnext: unlock the phone\n"),
            (
                "phone setup failed".to_owned(),
                Some("unlock the phone".to_owned())
            )
        );
    }
}
