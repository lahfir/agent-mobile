//! Session assembly: `AndroidAdapter` resolves tools and repo paths, then
//! `start_session` runs the ordered install → provision → enable →
//! forward → probe → bridge sequence, owning exactly one forward and one
//! bridge for its whole life.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_mobile_core::error::Failure;

use crate::adb::{Adb, resolve_sdk};
use crate::boot::boot_avd;
use crate::device::{self, AndroidScan, BootedAvd};
use crate::driver::{SecretToken, enable_service, ensure_apk, install, probe_status, provision};
use crate::forward::{create_forward, remove_forward};
use crate::http::{Bridge, start_bridge};
use crate::lifecycle::AdbLifecycle;

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

    /// Ordered session bring-up for `serial`: state check → APK → install
    /// → provision → enable → forward → status probe → bridge. Each later
    /// step's failure unwinds what this call created.
    ///
    /// # Errors
    /// [`Failure::Local`] at any stage; offline/unauthorized states carry
    /// remedies.
    pub fn start_session(&self, serial: &str) -> Result<AndroidSession, Failure> {
        check_device_state(&self.adb, serial)?;
        let apk = ensure_apk(
            self.apk_override.as_deref(),
            &self.driver_dir,
            self.adb.runner(),
        )?;
        install(&self.adb, serial, &apk)?;
        let token = provision(&self.adb, serial)?;
        enable_service(&self.adb, serial)?;
        let forward_port = create_forward(&self.adb, serial)?;
        self.finish_session(serial, apk, token, forward_port)
    }

    /// Probe + bridge after the forward exists; failures remove only this
    /// forward.
    fn finish_session(
        &self,
        serial: &str,
        apk: PathBuf,
        token: SecretToken,
        forward_port: u16,
    ) -> Result<AndroidSession, Failure> {
        let result = (|| {
            probe_status(&format!("http://127.0.0.1:{forward_port}"), &token)?;
            let lifecycle = Arc::new(AdbLifecycle::new(self.adb.clone(), serial));
            let bridge = start_bridge(forward_port, &token, lifecycle)?;
            let url = format!("http://127.0.0.1:{}", bridge.port());
            Ok((bridge, url))
        })();
        match result {
            Ok((bridge, url)) => Ok(AndroidSession {
                adb: self.adb.clone(),
                serial: serial.to_owned(),
                url,
                token,
                local_port: bridge.port(),
                forward_port,
                apk_source: apk,
                bridge: Some(bridge),
                closed: false,
            }),
            Err(e) => {
                let _ = remove_forward(&self.adb, serial, forward_port);
                Err(e)
            }
        }
    }
}

/// `adb get-state` must answer exactly `device`; the command exits nonzero
/// for offline/unauthorized, so success is not required — only the state
/// word matters.
fn check_device_state(adb: &Adb, serial: &str) -> Result<(), Failure> {
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
        format!("{serial} reports state {:?}", text.trim()),
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
            format!("adb version failed: {}", out.stderr),
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
    apk_source: PathBuf,
    bridge: Option<Bridge>,
    closed: bool,
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

    /// APK path that was installed.
    #[must_use]
    pub fn apk_source(&self) -> &Path {
        &self.apk_source
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
        match remove_forward(&self.adb, &self.serial, self.forward_port) {
            Ok(()) => {
                self.closed = true;
                Ok(())
            }
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
        let _ = remove_forward(&self.adb, &self.serial, self.forward_port);
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

#[cfg(test)]
mod tests;
