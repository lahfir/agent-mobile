//! Android-only runtime helpers kept pure so tests need no `adb`: serial
//! resolution, state-entry metadata, and stale-row cleanup.

use std::path::Path;
use std::time::Duration;

use agent_mobile_android::{AndroidAdapter, AndroidDeviceState, ForwardJournal};
use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

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

#[derive(Clone, Copy)]
pub(crate) struct AndroidMeta<'a> {
    pub(crate) serial: Option<&'a str>,
    pub(crate) forward_port: Option<u16>,
    pub(crate) device_port: Option<u16>,
    pub(crate) bridge_port: Option<u16>,
    pub(crate) apk_source: Option<&'a Path>,
    pub(crate) emulator_pid: Option<u32>,
}

/// Fill `e` with the Android cleanup metadata — the pure half of
/// `session_entry`, kept separate so tests can verify it without a live
/// session.
pub(crate) fn android_entry_fields(e: &mut SessionEntry, meta: AndroidMeta<'_>, log: &Path) {
    e.serial = meta.serial.map(str::to_owned);
    e.forward_port = meta.forward_port;
    e.device_port = meta.device_port;
    e.bridge_port = meta.bridge_port;
    e.apk_source = meta.apk_source.map(|p| p.to_string_lossy().into_owned());
    e.emulator_pid = meta.emulator_pid;
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
                "inspect `adb -s <serial> forward --list`, then remove the session row with \
                 `adb -s <serial> forward --remove tcp:<port>`",
            ));
        };
        return AndroidAdapter::from_environment()?.remove_owned_forward(
            serial,
            port,
            android_device_port(entry),
        );
    }
    if let Some(p) = entry.platform.as_deref().filter(|p| *p != "ios") {
        return Err(Failure::local(
            format!("session entry has unrecognized platform {p:?}"),
            "inspect the row in ~/.agent-mobile/state.json and fix or remove it",
        ));
    }
    reap_stale_runner(
        entry,
        agent_mobile_core::process::pid_alive,
        agent_mobile_core::process::terminate_runner,
        agent_mobile_core::process::await_exit,
    )
}

/// Prove-and-TERM a live stale runner, then require exit within 5s —
/// a live pid that cannot be proven, signalled, or reaped is a `Failure`
/// with the exact manual remedy, never a silent skip (KTD17).
pub(crate) fn reap_stale_runner(
    entry: &SessionEntry,
    alive: impl Fn(u32) -> bool,
    terminate: impl Fn(u32) -> bool,
    await_exit: impl Fn(u32, Duration) -> bool,
) -> Result<(), Failure> {
    let Some(rpid) = entry.runner_pid else {
        return Ok(());
    };
    if !alive(rpid) {
        return Ok(());
    }
    if !terminate(rpid) {
        return Err(Failure::local(
            format!("stale runner pid {rpid} is live but could not be proven and terminated"),
            format!(
                "verify `ps -o comm= -p {rpid}` shows xcodebuild, then `kill {rpid}` and retry"
            ),
        ));
    }
    if !await_exit(rpid, Duration::from_secs(5)) {
        return Err(Failure::local(
            format!("stale runner pid {rpid} did not exit within 5s of SIGTERM"),
            format!("run `kill -KILL {rpid}` and retry"),
        ));
    }
    Ok(())
}

/// Device-side loopback port recorded for the entry, defaulting to the
/// legacy fixed 8770 for rows written before dynamic ports existed.
pub(crate) fn android_device_port(entry: &SessionEntry) -> u16 {
    entry
        .device_port
        .unwrap_or(agent_mobile_android::LEGACY_DEVICE_PORT)
}

/// State-backed [`ForwardJournal`]: the pending-forward rows live in
/// `state.json` so a dead owner's allocations are reclaimable.
pub(crate) struct StateForwardJournal {
    store: StateStore,
}

impl StateForwardJournal {
    /// Journal over `store` (cloned handle; mutations go through the state
    /// lock).
    #[must_use]
    pub(crate) fn new(store: &StateStore) -> Self {
        Self {
            store: store.clone(),
        }
    }
}

impl ForwardJournal for StateForwardJournal {
    fn record(&self, serial: &str, local_port: u16, device_port: u16) -> Result<(), Failure> {
        self.store
            .record_pending_forward(serial, local_port, device_port)
    }

    fn clear(&self, serial: &str, local_port: u16, device_port: u16) -> Result<(), Failure> {
        self.store
            .clear_pending_forward(serial, local_port, device_port)
    }
}

/// Reclaim forwards recorded by owners that are provably gone — every
/// dead record's exact `serial`/`local`/`device` row is removed before
/// the record is dropped; live owners are left alone.
///
/// # Errors
/// [`Failure::Local`] when cleanup fails for a record on
/// `selected_serial`; unrelated failures are notes and retain the record.
pub fn sweep_pending_forwards(
    store: &StateStore,
    selected_serial: Option<&str>,
) -> Result<(), Failure> {
    let pending = store.pending_forwards();
    if pending.is_empty() {
        return Ok(());
    }
    let adapter = match AndroidAdapter::from_environment() {
        Ok(a) => a,
        Err(e) => {
            let selected_dead = pending.iter().any(|r| {
                selected_serial == Some(r.serial.as_str())
                    && !agent_mobile_core::process::process_matches(
                        r.owner_pid,
                        Some(&r.owner_started_at),
                    )
            });
            if selected_dead {
                return Err(e);
            }
            eprintln!("note: pending-forward sweep skipped: {}", e.message());
            return Ok(());
        }
    };
    sweep_pending_with(store, selected_serial, |rec| {
        adapter.remove_owned_forward(rec.serial.as_str(), rec.local_port, rec.device_port)
    })
}

/// `sweep_pending_forwards` with the removal injected — dead-owner
/// records remove only their exact tuple and then the record; a live
/// owner is skipped; the record survives a failed removal. Failures are
/// fatal only on `selected_serial`.
pub(crate) fn sweep_pending_with(
    store: &StateStore,
    selected_serial: Option<&str>,
    remove: impl Fn(&agent_mobile_core::state::PendingForward) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let pending = store.pending_forwards();
    for rec in pending {
        if agent_mobile_core::process::process_matches(rec.owner_pid, Some(&rec.owner_started_at)) {
            continue;
        }
        let removed = remove(&rec);
        match removed {
            Ok(()) => {
                store.remove_pending_forward(&rec)?;
            }
            Err(e) if selected_serial == Some(rec.serial.as_str()) => {
                return Err(Failure::local(
                    format!(
                        "stale pending forward {} tcp:{} tcp:{} cleanup failed: {}",
                        rec.serial,
                        rec.local_port,
                        rec.device_port,
                        e.message()
                    ),
                    e.render(),
                ));
            }
            Err(e) => {
                eprintln!(
                    "note: stale pending forward {} tcp:{} not reclaimed: {}",
                    rec.serial,
                    rec.local_port,
                    e.message()
                );
            }
        }
    }
    Ok(())
}
