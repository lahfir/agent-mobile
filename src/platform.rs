//! Normalized device layer (KTD10): one `PlatformDevice` covering iOS
//! simulators/devices and Android emulator/USB/wireless/AVD targets, so
//! verbs, `serve`, and lazy boot never branch on platform. Only this layer
//! imports `agent_mobile_core::ios` or `agent_mobile_android`.

mod android_ops;
mod discovery;
mod runtime;

pub use android_ops::cleanup_stale;
pub use discovery::discover;
pub use runtime::{PlatformRuntime, RuntimeExit};

use agent_mobile_android::{AndroidDeviceKind, AndroidDeviceState, AndroidTarget};
use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;

/// Which backend owns a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// iOS simulator or paired device.
    Ios,
    /// Android emulator, USB, or wireless target.
    Android,
}

impl Platform {
    /// Lowercase CLI label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }
}

/// Private platform payload behind the normalized seams.
#[derive(Debug, Clone)]
enum Backend {
    /// An iOS `simctl`/`devicectl` device.
    Ios(ios::Device),
    /// An `adb` target.
    Android(AndroidTarget),
}

/// One reachable or configured device, platform-agnostic.
#[derive(Debug, Clone)]
pub struct PlatformDevice {
    backend: Backend,
}

impl PlatformDevice {
    /// Wrap an iOS device.
    pub(crate) fn from_ios(d: ios::Device) -> Self {
        Self {
            backend: Backend::Ios(d),
        }
    }

    /// Wrap an Android target.
    pub(crate) fn from_android(t: AndroidTarget) -> Self {
        Self {
            backend: Backend::Android(t),
        }
    }

    /// iOS payload for runtime paths that own the xcodebuild runner.
    pub(crate) fn ios_device(&self) -> Option<&ios::Device> {
        match &self.backend {
            Backend::Ios(d) => Some(d),
            Backend::Android(_) => None,
        }
    }

    /// Android payload for runtime paths that own bridge/forward.
    pub(crate) fn android_target(&self) -> Option<&AndroidTarget> {
        match &self.backend {
            Backend::Ios(_) => None,
            Backend::Android(t) => Some(t),
        }
    }

    /// Owning platform.
    #[must_use]
    pub fn platform(&self) -> Platform {
        match &self.backend {
            Backend::Ios(_) => Platform::Ios,
            Backend::Android(_) => Platform::Android,
        }
    }

    /// Stable id: UDID for iOS; `avd:<name>` or serial for Android.
    #[must_use]
    pub fn id(&self) -> &str {
        match &self.backend {
            Backend::Ios(d) => &d.udid,
            Backend::Android(t) => &t.id,
        }
    }

    /// Collision-free state key: `<platform>:<id>`.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}", self.platform().as_str(), self.id())
    }

    /// Display name.
    #[must_use]
    pub fn name(&self) -> &str {
        match &self.backend {
            Backend::Ios(d) => &d.name,
            Backend::Android(t) => &t.name,
        }
    }

    /// Attachment kind: `simulator`/`device` or `emulator`/`usb`/`wireless`.
    #[must_use]
    pub fn kind(&self) -> &str {
        match &self.backend {
            Backend::Ios(d) => d.kind,
            Backend::Android(t) => android_kind(&t.kind),
        }
    }

    /// OS version when known.
    #[must_use]
    pub fn os(&self) -> Option<&str> {
        match &self.backend {
            Backend::Ios(d) => d.os.as_deref(),
            Backend::Android(t) => t.os.as_deref(),
        }
    }

    /// Boot/attach state when reported.
    #[must_use]
    pub fn state(&self) -> Option<&str> {
        match &self.backend {
            Backend::Ios(d) => d.state.as_deref(),
            Backend::Android(t) => Some(android_state(&t.state)),
        }
    }

    /// P1 state key — the iOS display name only; Android has none.
    #[must_use]
    pub fn legacy_key(&self) -> Option<&str> {
        match &self.backend {
            Backend::Ios(d) => Some(&d.name),
            Backend::Android(_) => None,
        }
    }

    /// The fixed `host:port` the iOS driver binds; Android forwards are
    /// ephemeral so it has none.
    #[must_use]
    pub fn fixed_addr(&self) -> Option<String> {
        match &self.backend {
            Backend::Ios(d) => Some(ios::driver_addr(d, ios::DEFAULT_PORT)),
            Backend::Android(_) => None,
        }
    }
}

/// Discovery result: devices plus non-fatal probe notes.
#[derive(Debug)]
pub struct PlatformScan {
    /// Every device both platforms reported.
    pub devices: Vec<PlatformDevice>,
    /// `ios:`/`android:`-prefixed probe failures worth surfacing.
    pub notes: Vec<String>,
}

/// `kind` label for an Android target.
fn android_kind(kind: &AndroidDeviceKind) -> &'static str {
    match kind {
        AndroidDeviceKind::Emulator => "emulator",
        AndroidDeviceKind::Usb => "usb",
        AndroidDeviceKind::Wireless => "wireless",
    }
}

/// State label for an Android target.
fn android_state(state: &AndroidDeviceState) -> &str {
    match state {
        AndroidDeviceState::Device => "device",
        AndroidDeviceState::Offline => "offline",
        AndroidDeviceState::Unauthorized => "unauthorized",
        AndroidDeviceState::Other(s) => s.as_str(),
    }
}

/// One matching rule of [`select`]: collision-free `key` wins outright,
/// then a unique stable `id` (UDID or serial/`avd:<name>`), then a unique
/// display name. Ambiguity at the id or name step is a usage error listing
/// every candidate's collision-free key, so an iPhone literally named
/// `emulator-5554` can never shadow the serial.
///
/// # Errors
/// [`Failure::Usage`] when the query is ambiguous or unknown.
pub fn select(scan: &PlatformScan, query: &str) -> Result<PlatformDevice, Failure> {
    if let Some(d) = scan.devices.iter().find(|d| d.key() == query) {
        return Ok(d.clone());
    }
    let ids: Vec<&PlatformDevice> = scan.devices.iter().filter(|d| d.id() == query).collect();
    match ids.len() {
        1 => return Ok(ids[0].clone()),
        0 => {}
        _ => return Err(ambiguous("id", query, &ids)),
    }
    let names: Vec<&PlatformDevice> = scan.devices.iter().filter(|d| d.name() == query).collect();
    match names.len() {
        1 => Ok(names[0].clone()),
        0 => Err(Failure::usage(format!(
            "unknown device {query:?}; run `agent-mobile devices`"
        ))),
        _ => Err(ambiguous("name", query, &names)),
    }
}

/// Discover then select — the serve/lazy resolution seam.
///
/// # Errors
/// [`Failure::Local`] when discovery fails entirely; [`Failure::Usage`]
/// when the query is ambiguous or unknown — except an `android:`/`avd:`/
/// `ios:`-prefixed miss that carries that platform's discovery note,
/// which surfaces the note and its toolchain remedy instead.
pub fn resolve(query: &str) -> Result<PlatformDevice, Failure> {
    let scan = discover()?;
    resolve_from(&scan, query)
}

/// Select inside an existing scan, rewriting an *unknown* stable-key miss
/// to the platform probe note when one was recorded — `android:foo` with
/// a broken SDK should say so, not claim the device is unknown.
pub(crate) fn resolve_from(scan: &PlatformScan, query: &str) -> Result<PlatformDevice, Failure> {
    select(scan, query).map_err(|e| contextualize(scan, query, e))
}

/// An "unknown device" usage failure on a platform-keyed query upgrades
/// to that side's recorded probe note plus its remedy; ambiguity and
/// note-less misses pass through unchanged.
fn contextualize(scan: &PlatformScan, query: &str, err: Failure) -> Failure {
    if !err.message().contains("unknown device") {
        return err;
    }
    let (prefix, remedy) = if query.starts_with("android:") || query.starts_with("avd:") {
        (
            "android:",
            "run `scripts/setup-android-sdk.sh --check`, then retry",
        )
    } else if query.starts_with("ios:") {
        ("ios:", "fix the Xcode toolchain, then retry")
    } else {
        return err;
    };
    scan.notes
        .iter()
        .find(|n| n.starts_with(prefix))
        .map_or(err, |n| {
            Failure::local(n.trim_start_matches(prefix).trim().to_owned(), remedy)
        })
}

/// The device a driverless lazy boot should pick, preserving the P1
/// mac order: iPhone-named simulator, any iOS simulator, an iOS physical
/// device, a ready Android target, then any Android target.
#[must_use]
pub fn default_device(scan: &PlatformScan) -> Option<PlatformDevice> {
    let ios_kind = |kind: &str| {
        scan.devices
            .iter()
            .find(|d| d.platform() == Platform::Ios && d.kind() == kind)
    };
    let android = |ready: bool| {
        scan.devices
            .iter()
            .find(|d| d.platform() == Platform::Android && (!ready || d.state() == Some("device")))
    };
    scan.devices
        .iter()
        .find(|d| {
            d.platform() == Platform::Ios && d.kind() == "simulator" && d.name().contains("iPhone")
        })
        .or_else(|| ios_kind("simulator"))
        .or_else(|| ios_kind("device"))
        .or_else(|| android(true))
        .or_else(|| android(false))
        .cloned()
}

/// Verbatim fragments of the certificate trust refusal and the
/// locked-device refusal; serve/lazy scan the driver log for them.
pub const TRUST_MARKERS: &[&str] = &[
    "not been explicitly trusted",
    "certificate is not trusted",
    "Developer App Certificate",
    "com.apple.dt.deviceprep",
    "to Continue",
];

/// Ambiguous-query failure naming every candidate's collision-free key.
fn ambiguous(what: &str, query: &str, hits: &[&PlatformDevice]) -> Failure {
    let keys = hits.iter().map(|d| d.key()).collect::<Vec<_>>().join(", ");
    Failure::usage(format!(
        "device {what} {query:?} is ambiguous — select a stable key: {keys}"
    ))
}

#[cfg(test)]
mod tests;
