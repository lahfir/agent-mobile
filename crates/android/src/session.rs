//! Session assembly: `AndroidAdapter` resolves tools and repo paths, then
//! `start_session` runs the ordered install → enable/bind → provision →
//! forward → probe → bridge sequence, owning exactly one forward and one
//! bridge for its whole life.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_mobile_core::error::Failure;

use crate::adb::{Adb, resolve_sdk};
use crate::boot::boot_avd;
use crate::device::{self, AndroidScan, BootedAvd};
use crate::driver::SecretToken;
use crate::forward::{ForwardJournal, remove_owned_forward};
use crate::http::Bridge;

/// Host-side entry point for Android sessions: resolved `adb`/`emulator`
/// binaries plus the repo's `drivers/android` directory.
pub struct AndroidAdapter {
    adb: Adb,
    emulator: PathBuf,
    driver_dir: PathBuf,
    apk_override: Option<PathBuf>,
}

/// Where `drivers/android` lives: `AGENT_MOBILE_REPO_ROOT` wins, else the
/// crate's `../../drivers/android` in a checkout.
fn driver_dir() -> PathBuf {
    if let Ok(root) = std::env::var("AGENT_MOBILE_REPO_ROOT") {
        return PathBuf::from(root).join("drivers/android");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../drivers/android")
}

/// `AGENT_MOBILE_ANDROID_APK` override, validated at use time.
fn apk_override() -> Option<PathBuf> {
    std::env::var("AGENT_MOBILE_ANDROID_APK")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

impl AndroidAdapter {
    /// Resolve the SDK tools (`ANDROID_HOME`, `ANDROID_SDK_ROOT`, platform
    /// default, PATH) and the driver checkout; validates `adb` answers.
    ///
    /// # Errors
    /// [`Failure::Local`] when `adb` cannot be found or does not answer a
    /// bounded `version` probe.
    pub fn from_environment() -> Result<Self, Failure> {
        let tools = resolve_sdk(|k| std::env::var(k).ok(), Path::is_file);
        let adb = Adb::real(tools.adb.clone());
        probe_tool(&adb)?;
        Ok(Self::for_test(adb, tools.emulator, apk_override()))
    }

    /// Injectable constructor for tests.
    pub(crate) fn for_test(adb: Adb, emulator: PathBuf, apk: Option<PathBuf>) -> Self {
        Self {
            adb,
            emulator,
            driver_dir: driver_dir(),
            apk_override: apk,
        }
    }

    /// Reachable devices plus configured AVDs.
    ///
    /// # Errors
    /// [`Failure::Local`] when `adb devices` cannot run.
    pub fn discover(&self) -> Result<AndroidScan, Failure> {
        device::discover(&self.adb, &self.emulator)
    }

    /// Boot `name` headless (or reuse a running instance), bounded by
    /// `budget`.
    ///
    /// # Errors
    /// [`Failure::Local`] on unsafe/unknown names, spawn failure, or boot
    /// timeouts with remedies.
    pub fn boot_avd(&self, name: &str, log: &Path, budget: Duration) -> Result<BootedAvd, Failure> {
        boot_avd(&self.adb, &self.emulator, name, log, budget)
    }

    /// [`AndroidAdapter::boot_avd`] honoring a cancellation flag; a
    /// cancelled boot leaves the emulator running for a later `serve`.
    ///
    /// # Errors
    /// Same contract as [`AndroidAdapter::boot_avd`] plus cancel.
    pub fn boot_avd_until(
        &self,
        name: &str,
        log: &Path,
        budget: Duration,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<BootedAvd, Failure> {
        crate::boot::boot_avd_until(&self.adb, &self.emulator, name, log, budget, cancelled)
    }

    /// Remove exactly the row `<serial> tcp:<local> tcp:<device_port>` —
    /// listed before and after so a foreign row is never removed and a
    /// surviving row is never reported clean.
    ///
    /// # Errors
    /// [`Failure::Local`] when the exact row survives removal.
    pub fn remove_owned_forward(
        &self,
        serial: &str,
        local_port: u16,
        device_port: u16,
    ) -> Result<(), Failure> {
        crate::forward::remove_owned_forward(&self.adb, serial, local_port, device_port)
    }
}

/// `adb get-state` must answer exactly `device`; the command exits nonzero
/// for offline/unauthorized, so success is not required — only the state
/// word matters.
pub(super) fn check_device_state(adb: &Adb, serial: &str) -> Result<(), Failure> {
    let out = adb.scoped(serial, &["get-state"])?;
    if out.success && out.stdout.trim() == "device" {
        return Ok(());
    }
    let text = format!("{} {}", out.stdout, out.stderr).to_lowercase();
    if text.contains("unauthorized") {
        return Err(Failure::local(
            format!("{serial} is unauthorized"),
            "accept the USB debugging prompt on the device and retry",
        ));
    }
    if text.contains("offline") {
        return Err(Failure::local(
            format!("{serial} is offline"),
            "run `adb reconnect` or replug the device and retry",
        ));
    }
    Err(Failure::local(
        format!(
            "{serial} reports state {:?}",
            crate::adb::bounded_diagnostic(text.trim())
        ),
        "check `adb devices` and retry",
    ))
}

/// `adb` answers a bounded `version` probe.
fn probe_tool(adb: &Adb) -> Result<(), Failure> {
    let out = adb.unscoped(&["version"])?;
    if out.success {
        Ok(())
    } else {
        Err(Failure::local(
            format!(
                "adb version failed: {}",
                crate::adb::diagnostic_output(&out)
            ),
            "install Android SDK platform-tools and retry",
        ))
    }
}

/// One live device session: bridge first (client-facing), prepared driver
/// and owned forward behind it. Drop/close stops only this session's
/// pieces.
pub struct AndroidSession {
    adb: Adb,
    serial: String,
    url: String,
    token: SecretToken,
    local_port: u16,
    forward_port: u16,
    device_port: u16,
    apk_source: PathBuf,
    bridge: Option<Bridge>,
    closed: bool,
    journal: Arc<dyn ForwardJournal>,
    journal_pending: bool,
}

impl AndroidSession {
    /// Client-facing bridge URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Bearer token for the session — in memory only.
    #[must_use]
    pub fn token(&self) -> &str {
        self.token.as_str()
    }

    /// `adb` serial this session owns.
    #[must_use]
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Bridge listen port on `127.0.0.1`.
    #[must_use]
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Owned `adb forward` host port.
    #[must_use]
    pub fn forward_port(&self) -> u16 {
        self.forward_port
    }

    /// Device-side loopback port the forward targets.
    #[must_use]
    pub fn device_port(&self) -> u16 {
        self.device_port
    }

    /// APK path that was installed.
    #[must_use]
    pub fn apk_source(&self) -> &Path {
        &self.apk_source
    }

    /// Whether the bridge is still serving — `false` once stopped or the
    /// accept thread ended.
    #[must_use]
    pub fn is_running(&self) -> bool {
        !self.closed && self.bridge.as_ref().is_some_and(Bridge::is_running)
    }

    /// Clear the pending-forward journal record after the session row is
    /// registered — the row itself is now the ownership record.
    ///
    /// # Errors
    /// [`Failure::Local`] when the journal clear fails; the record is
    /// retained for a later retry.
    pub fn commit_forward_journal(&mut self) -> Result<(), Failure> {
        if !self.journal_pending {
            return Ok(());
        }
        self.journal
            .clear(&self.serial, self.forward_port, self.device_port)?;
        self.journal_pending = false;
        Ok(())
    }

    /// Stop the bridge, then remove only this session's
    /// `forward --remove tcp:<port>` — accessibility stays enabled, the
    /// APK stays installed, other forwards are untouched.
    ///
    /// # Errors
    /// Propagates the forward-removal failure.
    pub fn close(mut self) -> Result<(), Failure> {
        if self.closed {
            return Ok(());
        }
        if let Some(mut bridge) = self.bridge.take() {
            bridge.stop();
        }
        match remove_owned_forward(
            &self.adb.without_cancellation(),
            &self.serial,
            self.forward_port,
            self.device_port,
        ) {
            Ok(()) => match self.commit_forward_journal() {
                Ok(()) => {
                    self.closed = true;
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    }
}

impl Drop for AndroidSession {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(mut bridge) = self.bridge.take() {
            bridge.stop();
        }
        let removed = remove_owned_forward(
            &self.adb.without_cancellation(),
            &self.serial,
            self.forward_port,
            self.device_port,
        );
        if removed.is_ok() {
            let _ = self.commit_forward_journal();
        }
    }
}

impl std::fmt::Debug for AndroidSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AndroidSession")
            .field("serial", &self.serial)
            .field("url", &self.url)
            .field("local_port", &self.local_port)
            .field("forward_port", &self.forward_port)
            .field("apk_source", &self.apk_source)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

mod startup;
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;
