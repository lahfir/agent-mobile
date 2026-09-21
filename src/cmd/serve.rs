//! `serve`: foreground driver for one device — takes the single-instance
//! lock, spawns the xcodebuild runner behind the drop guard, prints the
//! token and URL once, and supervises until the runner dies (KTD7, KTD8).

use std::fs::OpenOptions;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::process::{ServeChild, mint_token, tcp_ready};
use agent_mobile_core::state::{SessionEntry, StateStore};
use agent_mobile_core::wire::Wire;

use super::{Ctx, Session};

/// How long `serve` waits for the runner to bind the port; a cold build can
/// take minutes, so the budget is generous and progress streams to stderr.
const BOOT_BUDGET: Duration = Duration::from_secs(240);

/// Verbatim fragments of the certificate trust refusal (Experiments 7, 8);
/// `lazy` scans the same log for them.
pub(crate) const TRUST_MARKERS: &[&str] = &[
    "not been explicitly trusted",
    "certificate is not trusted",
    "Developer App Certificate",
];

/// What `await_driver` observed while the runner came up.
enum Boot {
    /// The port accepts TCP; the driver is serving.
    Ready,
    /// The log carried the trust refusal; the human must re-trust.
    TrustRefused,
    /// The runner exited before binding.
    Died,
    /// Nothing happened inside the budget.
    Timeout,
}

/// Run `serve <device>`; blocks for the life of the driver.
pub fn run(ctx: &Ctx, device_name: &str) -> Result<i32, Failure> {
    let store = &ctx.store;
    crate::cmd::ensure_state_dir(store)?;
    let lock = acquire_lock(store)?;
    let device = ios::find_device(device_name)?.ok_or_else(|| {
        Failure::usage(format!(
            "unknown device {device_name:?}; run `agent-mobile devices`"
        ))
    })?;
    reclaim_or_conflict(store, &device)?;
    if device.kind == "simulator" {
        eprintln!("booting {} ({})…", device.name, device.udid);
        ios::boot_simulator(&device.udid)?;
    }
    let token = mint_token()?;
    let token_file = StateStore::token_file_for(&device.name);
    store.remove_token(&token_file)?;
    store.write_token(&token_file, &token)?;
    let source = ios::driver_source()?;
    let log = store.driver_log(&device.name);
    let mut cmd = ios::serve_command(&device, ios::DEFAULT_PORT, &token, &source)?;
    let mut child = ServeChild::spawn_logged(&mut cmd, &log)?;
    eprintln!(
        "starting driver for {} — log: {}",
        device.name,
        log.display()
    );
    let url = ios::driver_url(&device, ios::DEFAULT_PORT);
    let addr = url.trim_start_matches("http://").to_owned();
    let booted = match await_driver(&mut child, &addr) {
        Boot::Ready => None,
        Boot::TrustRefused => Some(Failure::local(
            "the development certificate is not trusted on the device",
            "on the device: Settings > General > VPN & Device Management > \
             trust your Developer App certificate, then rerun `serve`",
        )),
        Boot::Died => Some(Failure::local(
            format!("the driver exited before binding; see {}", log.display()),
            "check the log for the failing step and retry `serve`",
        )),
        Boot::Timeout => Some(Failure::local(
            format!(
                "the driver did not bind {addr} within {}s",
                BOOT_BUDGET.as_secs()
            ),
            format!("check the log at {} and retry `serve`", log.display()),
        )),
    };
    if let Some(failure) = booted {
        let _ = store.remove_token(&token_file);
        return Err(failure);
    }
    let mut entry = SessionEntry::new(url.clone(), std::process::id(), token_file);
    entry.runner_pid = Some(child.pid());
    store.upsert(&device.name, &entry)?;
    store.remember_device(&device.name)?;
    super::emit(&format!(
        "url={url} token={token} device=\"{}\"",
        device.name
    ));
    eprintln!("driver ready on {url}");
    if let Some(app) = &ctx.app {
        let session = Session {
            wire: Wire::new(&url, &token),
            device: Some(device.name.clone()),
            last_snapshot_id: None,
        };
        let reply = session
            .wire
            .call("launch", &serde_json::json!({ "bundle_id": app }))?;
        let code = ctx.finish(&session, reply.envelope);
        if code != 0 {
            return Ok(code);
        }
    }
    supervise(store, &device, &mut child, lock)
}

/// The single-instance lock; a held lock reports the live session, the port,
/// and the remedy — never kill-by-port (KTD17).
fn acquire_lock(store: &StateStore) -> Result<std::fs::File, Failure> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(store.serve_lock_file())?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            let state = store.load();
            let detail = state
                .devices
                .iter()
                .next()
                .map_or_else(String::new, |(name, e)| {
                    format!(" (serving {name} on {}, pid {})", e.url, e.pid)
                });
            Err(Failure::local(
                format!("a driver is already running{detail}"),
                "use the live session, or `kill` its pid to stop it",
            ))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(Failure::from(e)),
    }
}

/// A live entry for this device means a foreign runner owns the port; a dead
/// one is reclaimed — a serve that died without cleanup (SIGKILL, panic) may
/// have left its runner holding the port, so the recorded owned pid gets a
/// TERM first. A foreign listener with no entry is named with the port, the
/// device, and the remedy (KTD7, KTD8, KTD17).
fn reclaim_or_conflict(store: &StateStore, device: &ios::Device) -> Result<(), Failure> {
    if let Some(entry) = store.entry(&device.name) {
        if agent_mobile_core::process::pid_alive(entry.pid) {
            return Err(Failure::local(
                format!(
                    "{} already has a driver on {} (pid {})",
                    device.name, entry.url, entry.pid
                ),
                "use the live session, or `kill` its pid to stop it",
            ));
        }
        if let Some(rpid) = entry.runner_pid
            && agent_mobile_core::process::pid_alive(rpid)
            && agent_mobile_core::process::terminate_runner(rpid)
        {
            let _ = agent_mobile_core::process::await_exit(rpid, Duration::from_secs(5));
        }
        store.remove(&device.name)?;
        store.remove_token(&entry.token_file)?;
    }
    let url = ios::driver_url(device, ios::DEFAULT_PORT);
    if tcp_ready(url.trim_start_matches("http://")) {
        return Err(Failure::local(
            format!(
                "port {} for {} is already bound by another process",
                ios::DEFAULT_PORT,
                device.name
            ),
            format!(
                "find the owner with `lsof -i :{}` and stop it, then retry `serve`",
                ios::DEFAULT_PORT
            ),
        ));
    }
    Ok(())
}

/// Poll the port, the log tail, and the child until one settles.
fn await_driver(child: &mut ServeChild, addr: &str) -> Boot {
    let deadline = Instant::now() + BOOT_BUDGET;
    while Instant::now() < deadline {
        if tcp_ready(addr) {
            return Boot::Ready;
        }
        let out = child.new_output();
        if TRUST_MARKERS.iter().any(|m| out.contains(m)) {
            return Boot::TrustRefused;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Boot::Died;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Boot::Timeout
}

/// Block on the runner; whatever ends it, the state entry goes with it.
/// SIGINT/SIGTERM forward to the child — the drop guard alone cannot run
/// under a signal, so the flag poll does the forwarding (KTD8).
fn supervise(
    store: &StateStore,
    device: &ios::Device,
    child: &mut ServeChild,
    lock: std::fs::File,
) -> Result<i32, Failure> {
    let _keep = lock;
    let token_file = StateStore::token_file_for(&device.name);
    let term = term_flag()?;
    let status = loop {
        if term.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = store.remove(&device.name);
            let _ = store.remove_token(&token_file);
            eprintln!("interrupted; driver stopped");
            return Ok(130);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = store.remove(&device.name);
    let _ = store.remove_token(&token_file);
    eprintln!("driver exited ({status})");
    Ok(i32::from(!status.success()))
}

/// Shared flag set by SIGINT/SIGTERM so `supervise` can forward the signal.
fn term_flag() -> Result<std::sync::Arc<std::sync::atomic::AtomicBool>, Failure> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, std::sync::Arc::clone(&flag))
        .and_then(|_| signal_hook::flag::register(SIGTERM, flag.clone()))
        .map_err(Failure::from)?;
    Ok(flag)
}
