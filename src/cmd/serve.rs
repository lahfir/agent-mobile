//! `serve`: foreground driver for one device — takes the single-instance
//! lock, spawns the xcodebuild runner behind the drop guard, prints the
//! token and URL once, and supervises until the runner dies (KTD7, KTD8).

use std::fs::OpenOptions;
use std::io::IsTerminal as _;
use std::path::Path;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::process::{BOOT_POLL, ServeChild, boot_budget, mint_token, tcp_ready};
use agent_mobile_core::state::{SessionEntry, StateStore};
use agent_mobile_core::wire::Wire;

use super::{Ctx, Session};

/// Verbatim fragments of the certificate trust refusal and the locked-device
/// refusal (Experiments 7-9); `lazy` scans the same log for them.
pub(crate) const TRUST_MARKERS: &[&str] = &[
    "not been explicitly trusted",
    "certificate is not trusted",
    "Developer App Certificate",
    "com.apple.dt.deviceprep",
    "to Continue",
];

/// Run `serve <device>`; blocks for the life of the driver.
pub fn run(ctx: &Ctx, device_name: &str) -> Result<i32, Failure> {
    let store = &ctx.store;
    crate::cmd::ensure_state_dir(store)?;
    let lock = acquire_lock(store)?;
    let term = term_flag()?;
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
    let addr = ios::driver_addr(&device, ios::DEFAULT_PORT);
    if let Err(failure) = await_driver(&mut child, &addr, &url, &token, &log, &term) {
        let _ = store.remove_token(&token_file);
        return Err(failure);
    }
    let mut entry = SessionEntry::new(url.clone(), std::process::id(), token_file.clone());
    entry.runner_pid = Some(child.pid());
    store.upsert(&device.name, &entry)?;
    store.remember_device(&device.name)?;
    let ready = ready_line(&url, &token, &device.name, store, &token_file);
    super::emit(&ready);
    eprintln!("driver ready on {url}");
    if let Some(app) = &ctx.app {
        let session = Session::new(url.clone(), token);
        let code = super::round_trip_within(
            ctx,
            &session,
            "launch",
            &serde_json::json!({ "bundle_id": app }),
            agent_mobile_core::wire::LONG_TIMEOUT,
        )?;
        if code != 0 {
            clear_session(store, &device.name, &token_file);
            return Ok(code);
        }
    }
    supervise(store, &device, &mut child, lock, &token_file, &term)
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
/// TERM first. A bound port with no live entry can still be ours: another
/// device's dead serve may have orphaned a runner holding it, so dead
/// entries across every device are swept before the port is called foreign.
/// A foreign listener is named with the port, the device, and the remedy
/// (KTD7, KTD8, KTD17).
fn reclaim_or_conflict(store: &StateStore, device: &ios::Device) -> Result<(), Failure> {
    let addr = ios::driver_addr(device, ios::DEFAULT_PORT);
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
        reap_entry(store, &device.name, &entry);
    }
    if !tcp_ready(&addr) {
        return Ok(());
    }
    let state = store.load();
    for (name, entry) in &state.devices {
        if name != &device.name && !agent_mobile_core::process::pid_alive(entry.pid) {
            reap_entry(store, name, entry);
        }
    }
    if tcp_ready(&addr) {
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

/// The one-time ready line. The bearer token only prints to a real terminal —
/// under lazy boot stdout is the append-mode driver log, and secrets do not
/// land there; a piped run points at the token file instead.
fn ready_line(
    url: &str,
    token: &str,
    device: &str,
    store: &StateStore,
    token_file: &str,
) -> String {
    if std::io::stdout().is_terminal() {
        format!("url={url} token={token} device=\"{device}\"")
    } else {
        format!(
            "url={url} device=\"{device}\" token_file={}",
            store.token_path(token_file).display()
        )
    }
}

/// Reap a dead session's orphaned runner, then drop the entry and token.
fn reap_entry(store: &StateStore, device: &str, entry: &SessionEntry) {
    if let Some(rpid) = entry.runner_pid
        && agent_mobile_core::process::pid_alive(rpid)
        && agent_mobile_core::process::terminate_runner(rpid)
    {
        let _ = agent_mobile_core::process::await_exit(rpid, Duration::from_secs(5));
    }
    let _ = store.remove(device);
    let _ = store.remove_token(&entry.token_file);
}

/// Poll the port, the log tail, and the child until one settles; `Ok` means
/// the freshly minted token got a `status` reply — a bare TCP accept is not
/// enough, since a foreign listener could hold the port instead. A listener
/// that answers the protocol but rejects our token, or answers with HTTP we
/// cannot parse, is a squatter and fails fast; only transport-level silence
/// keeps polling. `addr` resolves once — mDNS lookups are the expensive part
/// of every poll — and retries only while resolution itself is failing.
fn await_driver(
    child: &mut ServeChild,
    addr: &str,
    url: &str,
    token: &str,
    log: &Path,
    term: &std::sync::atomic::AtomicBool,
) -> Result<(), Failure> {
    use std::net::{SocketAddr, ToSocketAddrs};
    use std::sync::atomic::Ordering;
    let deadline = Instant::now() + boot_budget();
    let probe = Wire::with_timeout(url, token, Duration::from_secs(3));
    let mut addrs: Option<Vec<SocketAddr>> = addr.to_socket_addrs().ok().map(Iterator::collect);
    while Instant::now() < deadline {
        if term.load(Ordering::Relaxed) {
            return Err(Failure::local(
                "interrupted during driver boot",
                "rerun `serve` to start the driver again",
            ));
        }
        if let Some(list) = &addrs
            && agent_mobile_core::process::tcp_ready_at(list)
        {
            match probe.call("status", &serde_json::json!({})) {
                Ok(env) if env.ok => return Ok(()),
                Ok(_) | Err(Failure::Local { .. } | Failure::Driver { .. }) => {
                    return Err(Failure::local(
                        format!("{addr} is held by a service that is not this driver"),
                        format!(
                            "find the owner with `lsof -i :{}` and stop it, then retry `serve`",
                            ios::DEFAULT_PORT
                        ),
                    ));
                }
                Err(_) => {}
            }
        }
        if addrs.is_none() {
            addrs = addr.to_socket_addrs().ok().map(Iterator::collect);
        }
        let out = child.new_output();
        if TRUST_MARKERS.iter().any(|m| out.contains(m)) {
            return Err(Failure::local(
                "the device refused the runner: certificate untrusted or device locked",
                "unlock the device and trust the Developer App certificate under \
                 Settings > General > VPN & Device Management, then rerun `serve`",
            ));
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Err(Failure::local(
                format!("the driver exited before binding; see {}", log.display()),
                "check the log for the failing step and retry `serve`",
            ));
        }
        std::thread::sleep(BOOT_POLL);
    }
    Err(Failure::local(
        format!(
            "the driver did not answer {addr} within {}s",
            boot_budget().as_secs()
        ),
        format!("check the log at {} and retry `serve`", log.display()),
    ))
}

/// Block on the runner; whatever ends it, the state entry goes with it.
/// SIGINT/SIGTERM forward to the child — the drop guard alone cannot run
/// under a signal, so the flag poll does the forwarding (KTD8).
fn supervise(
    store: &StateStore,
    device: &ios::Device,
    child: &mut ServeChild,
    _lock: std::fs::File,
    token_file: &str,
    term: &std::sync::atomic::AtomicBool,
) -> Result<i32, Failure> {
    let status = loop {
        if term.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            clear_session(store, &device.name, token_file);
            eprintln!("interrupted; driver stopped");
            return Ok(130);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    clear_session(store, &device.name, token_file);
    eprintln!("driver exited ({status})");
    Ok(i32::from(!status.success()))
}

/// Drop the session entry and its token file; both best-effort — the serve
/// is going down either way.
fn clear_session(store: &StateStore, device: &str, token_file: &str) {
    let _ = store.remove(device);
    let _ = store.remove_token(token_file);
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
