//! `serve`: foreground driver for one device on either platform — takes
//! the single-instance lock, reclaims stale state, starts a
//! `PlatformRuntime`, prints the token and URL once, and supervises until
//! the runtime dies (KTD7, KTD8).

use std::fs::OpenOptions;
use std::io::IsTerminal as _;
use std::time::Duration;

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

use super::{Ctx, Session};
use crate::platform::{self, PlatformDevice, PlatformRuntime};

/// Run `serve <device>`; blocks for the life of the driver.
pub fn run(ctx: &Ctx, device_name: &str) -> Result<i32, Failure> {
    let store = &ctx.store;
    crate::cmd::ensure_state_dir(store)?;
    let lock = acquire_lock(store)?;
    let term = term_flag()?;
    let device = platform::resolve(device_name)?;
    let key = device.key();
    reclaim_or_conflict(store, &device)?;
    let log = store.driver_log(&key);
    let mut runtime = PlatformRuntime::start(&device, &log, &term)?;
    let token_file = StateStore::token_file_for(&key);
    let entry = runtime.session_entry(token_file.clone(), &log);
    if let Err(f) = register_session(store, &key, &token_file, &entry, runtime.token()) {
        return finish_cleanup(store, &key, &token_file, &entry, Err(f), || runtime.stop());
    }
    let ready = ready_line(&runtime, &device, store, &token_file);
    super::emit(&ready);
    eprintln!("driver ready on {}", runtime.url());
    if let Some(app) = &ctx.app {
        let session = Session::new(runtime.url().to_owned(), runtime.token().to_owned());
        let launch = super::round_trip_within(
            ctx,
            &session,
            "launch",
            &serde_json::json!({ "bundle_id": app }),
            agent_mobile_core::wire::LONG_TIMEOUT,
        );
        match launch {
            Ok(0) => {}
            other => {
                return finish_cleanup(store, &key, &token_file, &entry, other, || runtime.stop());
            }
        }
    }
    supervise(store, &key, &mut runtime, lock, &token_file, &term, &entry)
}

/// The single-instance lock; a held lock reports the live session, the
/// port, and the remedy — never kill-by-port (KTD17).
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

/// Reclaim state for the selected device and sweep every dead row. A live
/// entry under either the collision-free key or the iOS legacy name is a
/// conflict. A dead selected row gets `cleanup_stale` — its failure
/// propagates verbatim so the named remedy stays executable. Other
/// devices' dead rows are swept best-effort: a failed unrelated Android
/// cleanup stays recorded and is only a note. Finally, iOS's fixed driver
/// port must be free once stale runners are reaped — a still-bound port
/// is a foreign listener (KTD7, KTD8, KTD17); Android's ephemeral forward
/// needs no such check.
fn reclaim_or_conflict(store: &StateStore, device: &PlatformDevice) -> Result<(), Failure> {
    let mut aliases = vec![device.key()];
    if let Some(legacy) = device.legacy_key() {
        aliases.push(legacy.to_owned());
    }
    for alias in &aliases {
        let Some(entry) = store.entry(alias) else {
            continue;
        };
        if agent_mobile_core::process::pid_alive(entry.pid) {
            return Err(Failure::local(
                format!(
                    "{} already has a driver on {} (pid {})",
                    device.name(),
                    entry.url,
                    entry.pid
                ),
                "use the live session, or `kill` its pid to stop it",
            ));
        }
        platform::cleanup_stale(&entry)?;
        let _ = store.remove(alias);
        let _ = store.remove_token(&entry.token_file);
    }
    let state = store.load();
    for (name, entry) in &state.devices {
        if aliases.iter().any(|a| a == name) || agent_mobile_core::process::pid_alive(entry.pid) {
            continue;
        }
        match platform::cleanup_stale(entry) {
            Ok(()) => {
                let _ = store.remove(name);
                let _ = store.remove_token(&entry.token_file);
            }
            Err(e) => {
                eprintln!("note: stale {name} session not reclaimed: {}", e.message());
            }
        }
    }
    if let Some(addr) = device.fixed_addr()
        && agent_mobile_core::process::tcp_ready(&addr)
    {
        return Err(Failure::local(
            format!(
                "{addr} for {} is already bound by another process",
                device.name()
            ),
            format!(
                "find the owner with `lsof -i :{}` and stop it, then retry `serve`",
                addr.rsplit(':').next().unwrap_or("8770")
            ),
        ));
    }
    Ok(())
}

/// Persist the session rows that make this runtime claimable: fresh
/// token file (0600), the precomputed state entry, then the remembered
/// device. Any failure after the runtime started routes through cleanup.
fn register_session(
    store: &StateStore,
    key: &str,
    token_file: &str,
    entry: &SessionEntry,
    token: &str,
) -> Result<(), Failure> {
    store.remove_token(token_file)?;
    store.write_token(token_file, token)?;
    store.upsert(key, entry)?;
    store.remember_device(key)
}

/// Drop the session entry and its token file; both best-effort — the
/// serve is going down either way.
fn clear_session(store: &StateStore, key: &str, token_file: &str) {
    let _ = store.remove(key);
    let _ = store.remove_token(token_file);
}

/// Close every serve error path through the same ordering: `stop` first,
/// state/token removal only after a successful stop. When the stop itself
/// fails, the already-computed `entry` is re-upserted best-effort — it is
/// the only exact serial/forward record, so the next `serve` can reclaim
/// rather than leak. Cleanup failure takes precedence over the original
/// error.
fn finish_cleanup(
    store: &StateStore,
    key: &str,
    token_file: &str,
    entry: &SessionEntry,
    result: Result<i32, Failure>,
    stop: impl FnOnce() -> Result<(), Failure>,
) -> Result<i32, Failure> {
    let cleanup = stop().map(|()| clear_session(store, key, token_file));
    match (result, cleanup) {
        (Ok(code), Ok(())) => Ok(code),
        (_, Err(cleanup_failure)) => {
            let _ = store.upsert(key, entry);
            Err(cleanup_failure)
        }
        (Err(original), Ok(())) => Err(original),
    }
}

/// The one-time ready line. The bearer token only prints to a real
/// terminal — under lazy boot stdout is the append-mode driver log, and
/// secrets do not land there; a piped run points at the token file
/// instead.
fn ready_line(
    runtime: &PlatformRuntime,
    device: &PlatformDevice,
    store: &StateStore,
    token_file: &str,
) -> String {
    let base = format!(
        "url={} device=\"{}\" platform={} id={}",
        runtime.url(),
        device.name(),
        device.platform().as_str(),
        device.id()
    );
    if std::io::stdout().is_terminal() {
        format!("{base} token={}", runtime.token())
    } else {
        format!(
            "{base} token_file={}",
            store.token_path(token_file).display()
        )
    }
}

/// Block on the runtime; whatever ends it, the state entry goes with it
/// once owned cleanup succeeds. SIGINT/SIGTERM forwards through `stop` —
/// the drop guard alone cannot run under a signal (KTD8). A `poll_exit`
/// failure takes the same cleanup path before the error surfaces.
fn supervise(
    store: &StateStore,
    key: &str,
    runtime: &mut PlatformRuntime,
    _lock: std::fs::File,
    token_file: &str,
    term: &std::sync::atomic::AtomicBool,
    entry: &SessionEntry,
) -> Result<i32, Failure> {
    loop {
        if term.load(std::sync::atomic::Ordering::Relaxed) {
            finish_cleanup(store, key, token_file, entry, Ok(130), || runtime.stop())?;
            eprintln!("interrupted; driver stopped");
            return Ok(130);
        }
        match runtime.poll_exit() {
            Ok(Some(crate::platform::RuntimeExit { success, detail })) => {
                finish_cleanup(
                    store,
                    key,
                    token_file,
                    entry,
                    Ok(i32::from(!success)),
                    || runtime.stop(),
                )?;
                eprintln!("driver exited ({detail})");
                return Ok(i32::from(!success));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(f) => {
                return finish_cleanup(store, key, token_file, entry, Err(f), || runtime.stop());
            }
        }
    }
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

#[cfg(test)]
mod tests;
