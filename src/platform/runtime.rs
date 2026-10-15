//! Platform runtime: one supervised driver backend — an iOS `xcodebuild`
//! child or an `AndroidSession` (bridge + owned forward) — behind the
//! identical `serve` lifecycle contract.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use agent_mobile_android::{AndroidAdapter, AndroidSession, AndroidSessionMeta};
use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::process::{BOOT_POLL, ServeChild, boot_budget, mint_token, tcp_ready_at};
use agent_mobile_core::state::{SessionEntry, StateStore};
use agent_mobile_core::wire::Wire;

use super::android_ops::{StateForwardJournal, android_entry_fields, android_serial};
use super::{Platform, PlatformDevice, TRUST_MARKERS};

/// An exited runtime's verdict.
#[derive(Debug)]
pub struct RuntimeExit {
    /// Whether the backend exited cleanly.
    pub success: bool,
    /// Human-readable ending (exit status or bridge stop).
    pub detail: String,
}

/// The owned backend: the iOS runner child or the Android session. `None`
/// only between `stop` taking it and drop.
enum Backend {
    /// xcodebuild runner `serve` spawned and must reap.
    Ios(ServeChild),
    /// Bridge + forward `AndroidSession` owns and must release.
    Android(Box<AndroidSession>),
}

/// Exact forward-removal record a failed `stop` keeps so Drop (or a
/// second `stop`) retries the same removal instead of losing it.
#[derive(Debug, Clone)]
struct PendingForwardRemoval {
    serial: String,
    local_port: u16,
    device_port: u16,
}

/// One running driver regardless of platform.
pub struct PlatformRuntime {
    backend: Option<Backend>,
    platform: Platform,
    device_id: String,
    device_name: Option<String>,
    url: String,
    token: String,
    android_meta: Option<AndroidSessionMeta>,
    retry_forward: Option<PendingForwardRemoval>,
    stopped: bool,
}

impl PlatformRuntime {
    /// Start a driver for `device`: iOS preserves the exact P1 serve path
    /// (boot → token → runner → bounded authenticated await); Android
    /// rejects unreachable rows, boots a shutdown AVD when that's the
    /// selected target, then runs journaled session bring-up. `log` receives the
    /// runner/emulator output; `term` cancels the wait.
    ///
    /// # Errors
    /// [`Failure::Local`] on boot, launch, or readiness failure.
    pub fn start(
        device: &PlatformDevice,
        log: &Path,
        term: &Arc<AtomicBool>,
        store: &StateStore,
    ) -> Result<Self, Failure> {
        match device.platform() {
            Platform::Ios => Self::start_ios(device, log, term),
            Platform::Android => Self::start_android(device, log, term, store),
        }
    }

    /// Clear the Android pending-forward journal record — call only after
    /// the session row is fully registered; the row then owns cleanup.
    ///
    /// # Errors
    /// [`Failure::Local`] when the journal clear fails.
    pub fn commit_startup(&mut self) -> Result<(), Failure> {
        match self.backend.as_mut() {
            Some(Backend::Android(session)) => session.commit_forward_journal(),
            _ => Ok(()),
        }
    }

    /// The exact P1 iOS bring-up, unchanged in behavior.
    fn start_ios(
        device: &PlatformDevice,
        log: &Path,
        term: &Arc<AtomicBool>,
    ) -> Result<Self, Failure> {
        let Some(d) = device.ios_device() else {
            return Err(Failure::local("not an iOS device", "report a bug"));
        };
        if d.kind == "simulator" {
            eprintln!("booting {} ({})…", d.name, d.udid);
            ios::boot_simulator(&d.udid)?;
        }
        let token = mint_token()?;
        let source = ios::driver_source()?;
        let mut cmd = ios::serve_command(d, ios::DEFAULT_PORT, &token, &source)?;
        let mut child = ServeChild::spawn_logged(&mut cmd, log)?;
        eprintln!("starting driver for {} — log: {}", d.name, log.display());
        let url = ios::driver_url(d, ios::DEFAULT_PORT);
        let addr = ios::driver_addr(d, ios::DEFAULT_PORT);
        await_ios_driver(&mut child, &addr, &url, &token, log, term)?;
        Ok(Self {
            backend: Some(Backend::Ios(child)),
            platform: Platform::Ios,
            device_id: d.udid.clone(),
            device_name: Some(device.name().to_owned()),
            url,
            token,
            android_meta: None,
            retry_forward: None,
            stopped: false,
        })
    }

    /// Android bring-up: state gate → optional AVD boot → session; the
    /// cancellation flag flows into `start_session_until` and is checked
    /// again between the probe/bridge boundaries.
    fn start_android(
        device: &PlatformDevice,
        log: &Path,
        term: &Arc<AtomicBool>,
        store: &StateStore,
    ) -> Result<Self, Failure> {
        let Some(target) = device.android_target() else {
            return Err(Failure::local("not an Android target", "report a bug"));
        };
        let adapter = AndroidAdapter::from_environment()?;
        let (serial, emulator_pid) = android_serial(target, |avd| {
            eprintln!("booting AVD {avd} — log: {}", log.display());
            adapter.boot_avd_until(avd, log, boot_budget(), term.as_ref())
        })?;
        if term.load(Ordering::Relaxed) {
            return Err(Failure::local("operation interrupted", "rerun the command"));
        }
        let session = adapter.start_session_until_journaled(
            &serial,
            term.clone(),
            Arc::new(StateForwardJournal::new(store)),
        )?;
        if term.load(Ordering::Relaxed) {
            match session.close() {
                Ok(()) => {
                    return Err(Failure::local("operation interrupted", "rerun the command"));
                }
                Err(cleanup) => return Err(cleanup),
            }
        }
        let android_meta = AndroidSessionMeta {
            serial,
            forward_port: session.forward_port(),
            device_port: session.device_port(),
            bridge_port: session.local_port(),
            apk_source: session.apk_source().to_path_buf(),
            emulator_pid,
        };
        Ok(Self {
            url: session.url().to_owned(),
            token: session.token().to_owned(),
            platform: Platform::Android,
            device_id: device.id().to_owned(),
            device_name: Some(device.name().to_owned()),
            android_meta: Some(android_meta),
            backend: Some(Backend::Android(Box::new(session))),
            retry_forward: None,
            stopped: false,
        })
    }

    /// Driver base URL clients should talk to.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Bearer token (in memory only).
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The state row describing this session: owner pid is always the
    /// serve process; iOS adds only `runner_pid`; Android adds only its
    /// cleanup metadata. The token never lands here.
    #[must_use]
    pub fn session_entry(&self, token_file: String, log: &Path) -> SessionEntry {
        let mut e = SessionEntry::new(self.url.clone(), std::process::id(), token_file);
        e.platform = Some(self.platform.as_str().to_owned());
        e.device_id = Some(self.device_id.clone());
        e.device_name.clone_from(&self.device_name);
        match &self.backend {
            Some(Backend::Ios(child)) => e.runner_pid = Some(child.pid()),
            Some(Backend::Android(_)) => {
                if let Some(meta) = &self.android_meta {
                    android_entry_fields(&mut e, meta, log);
                }
            }
            None => {}
        }
        e
    }

    /// `Some` once the backend ended: the iOS child's exit, or the Android
    /// bridge thread finishing (which only happens on failure — `stop`
    /// runs in-band).
    ///
    /// # Errors
    /// [`Failure::Local`] when the iOS child status cannot be polled.
    pub fn poll_exit(&mut self) -> Result<Option<RuntimeExit>, Failure> {
        match self.backend.as_mut() {
            Some(Backend::Ios(child)) => Ok(child.try_wait()?.map(|s| RuntimeExit {
                success: s.success(),
                detail: format!("{s}"),
            })),
            Some(Backend::Android(session)) => {
                if session.is_running() {
                    Ok(None)
                } else {
                    Ok(Some(RuntimeExit {
                        success: false,
                        detail: "the Android bridge stopped".to_owned(),
                    }))
                }
            }
            None => Ok(None),
        }
    }

    /// Release only what this runtime owns: kill+wait the iOS child, or
    /// close the Android session (bridge first, then the exact forward).
    /// `stopped` flips only after a fully successful cleanup — a failed
    /// Android close leaves the recorded removal behind so a second
    /// `stop` (or Drop) retries the exact forward rather than losing the
    /// ownership record.
    ///
    /// # Errors
    /// Propagates the Android forward-removal failure.
    pub fn stop(&mut self) -> Result<(), Failure> {
        self.stop_with(|serial, port, device_port| {
            AndroidAdapter::from_environment()?.remove_owned_forward(serial, port, device_port)
        })
    }

    /// [`stop`] with the forward-removal seam injectable for tests: the
    /// state machine — fresh → failed-close (retry recorded) → retried
    /// exact removal — is identical either way.
    pub(crate) fn stop_with(
        &mut self,
        remove: impl FnOnce(&str, u16, u16) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        if self.stopped {
            return Ok(());
        }
        match self.backend.take() {
            Some(Backend::Ios(mut child)) => {
                if child.try_wait()?.is_none() {
                    child.kill()?;
                }
                child.wait()?;
                self.stopped = true;
                Ok(())
            }
            Some(Backend::Android(session)) => match session.close() {
                Ok(()) => {
                    self.stopped = true;
                    self.retry_forward = None;
                    Ok(())
                }
                Err(e) => {
                    self.retry_forward =
                        self.android_meta.as_ref().map(|m| PendingForwardRemoval {
                            serial: m.serial.clone(),
                            local_port: m.forward_port,
                            device_port: m.device_port,
                        });
                    Err(e)
                }
            },
            None => {
                let Some(rec) = self.retry_forward.clone() else {
                    self.stopped = true;
                    return Ok(());
                };
                match remove(&rec.serial, rec.local_port, rec.device_port) {
                    Ok(()) => {
                        self.retry_forward = None;
                        self.stopped = true;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }
}

impl Drop for PlatformRuntime {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Poll the port, the log tail, and the child until one settles; `Ok`
/// means the freshly minted token got a `status` reply — a bare TCP
/// accept is not enough, since a foreign listener could hold the port
/// instead. A listener that answers the protocol but rejects our token,
/// or answers with HTTP we cannot parse, is a squatter and fails fast;
/// only transport-level silence keeps polling. `addr` resolves once —
/// mDNS lookups are the expensive part of every poll — and retries only
/// while resolution itself is failing.
fn await_ios_driver(
    child: &mut ServeChild,
    addr: &str,
    url: &str,
    token: &str,
    log: &Path,
    term: &AtomicBool,
) -> Result<(), Failure> {
    use std::net::{SocketAddr, ToSocketAddrs};
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
            && tcp_ready_at(list)
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
