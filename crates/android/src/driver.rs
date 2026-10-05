//! On-device driver setup: locate or build the debug APK, install it,
//! provision a bearer token that lives only in memory, merge-enable the
//! accessibility service without disturbing existing entries, and create a
//! verified owned `adb forward`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::wire::Wire;
use serde_json::json;

use crate::adb::{Adb, BUILD_TIMEOUT, CommandRunner, INSTALL_TIMEOUT, failed_op};

/// Driver application id.
pub const PACKAGE: &str = "com.lahfir.agentmobile.driver";
/// Accessibility service component as `settings secure` stores it.
pub const SERVICE_COMPONENT: &str =
    "com.lahfir.agentmobile.driver/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService";
/// Content-provider authority that mints session tokens.
pub const PROVIDER_URI: &str = "content://com.lahfir.agentmobile.driver.provision";
/// Device-side HTTP listener the forward points at.
pub const DEVICE_PORT: u16 = 8770;

/// Token length minted by the provider: 32 bytes base64url, no padding.
const TOKEN_LEN: usize = 43;

/// How long to poll `dumpsys accessibility` for the service bind.
const BIND_BUDGET: Duration = Duration::from_secs(20);

/// A provisioned bearer token: exists only in memory, never in logs or
/// error text. Debug and Display both render `<redacted>`.
#[derive(Clone)]
pub(crate) struct SecretToken(String);

impl SecretToken {
    /// Wrap an already-minted token string; only tests mint directly.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new(token: &str) -> Self {
        Self(token.to_owned())
    }

    /// Raw token for in-memory wire use only.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Boundary characters that may precede a `token` field name in
/// `Bundle[{...}]` output.
fn is_boundary(byte: u8) -> bool {
    matches!(byte, b'{' | b',' | b' ' | b'\t' | b'\n')
}

/// Is `byte` in the base64url alphabet?
fn is_b64url(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// Extract the single `token=<43 base64url>` field: the name must sit at a
/// field boundary (not `not_token=`), the value must be exactly
/// [`TOKEN_LEN`] base64url bytes followed by a non-base64url delimiter or
/// end, and exactly one such field may appear.
fn extract_token(output: &str) -> Option<String> {
    let bytes = output.as_bytes();
    let mut found: Option<&str> = None;
    let mut pos = 0;
    while let Some(rel) = output[pos..].find("token=") {
        let start = pos + rel;
        pos = start + "token=".len();
        if start > 0 && !is_boundary(bytes[start - 1]) {
            continue;
        }
        let value_end = pos + TOKEN_LEN;
        if value_end > bytes.len() {
            return None;
        }
        let value = &output[pos..value_end];
        let exact = value.bytes().all(is_b64url)
            && (value_end == bytes.len() || !is_b64url(bytes[value_end]));
        if !exact || found.replace(value).is_some() {
            return None;
        }
    }
    found.map(str::to_owned)
}

/// `content call --uri PROVIDER_URI --method provision`; the token never
/// touches argv, env, or logs.
///
/// # Errors
/// [`Failure::Local`] when the provider refuses or returns no token; the
/// message never embeds provider output.
pub(crate) fn provision(adb: &Adb, serial: &str) -> Result<SecretToken, Failure> {
    let out = adb.scoped(
        serial,
        &[
            "shell",
            "content",
            "call",
            "--uri",
            PROVIDER_URI,
            "--method",
            "provision",
        ],
    )?;
    if !out.success {
        return Err(Failure::local(
            format!("driver provision call refused on {serial}"),
            "reinstall the driver APK and retry",
        ));
    }
    extract_token(&out.stdout).map(SecretToken).ok_or_else(|| {
        Failure::local(
            "provision returned no token",
            "check the driver service is installed and retry",
        )
    })
}

/// Merge our service component into a `settings secure` value, preserving
/// existing colon-separated entries in order and never duplicating.
#[must_use]
pub(crate) fn merge_enabled_services(raw: &str, ours: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for piece in raw.split(':').map(str::trim) {
        if piece.is_empty() || piece == "null" {
            continue;
        }
        if !parts.iter().any(|p| p == piece) {
            parts.push(piece.to_owned());
        }
    }
    let present = parts.iter().any(|p| p == ours)
        || (ours == SERVICE_COMPONENT && parts.iter().any(|p| is_our_service_entry(p)));
    if !present {
        parts.push(ours.to_owned());
    }
    parts.join(":")
}

/// Is this colon-list entry exactly our service, full or short form?
fn is_our_service_entry(entry: &str) -> bool {
    let Some((pkg, class)) = entry.trim().split_once('/') else {
        return false;
    };
    pkg == PACKAGE
        && (entry.trim() == SERVICE_COMPONENT || class == ".AgentMobileAccessibilityService")
}

/// Bounded `settings get/put` helpers.
fn settings_get(adb: &Adb, serial: &str, key: &str) -> Result<String, Failure> {
    let out = adb.scoped_ok(
        serial,
        "settings read",
        &["shell", "settings", "get", "secure", key],
    )?;
    Ok(out.stdout.trim().to_owned())
}

fn settings_put(adb: &Adb, serial: &str, key: &str, value: &str) -> Result<(), Failure> {
    adb.scoped_ok(
        serial,
        "settings write",
        &["shell", "settings", "put", "secure", key, value],
    )
    .map(|_| ())
    .map_err(|_| bind_failure("settings write was refused"))
}

/// Enable and verify the accessibility service: writes happen only when
/// the merged value differs, then a reread and a bounded `dumpsys` poll
/// confirm the service actually bound.
///
/// # Errors
/// [`Failure::Local`] when writes fail, the service does not appear in
/// `enabled_accessibility_services`, or it never binds; the refusal names
/// Settings > Accessibility and App Info > Allow restricted settings.
pub(crate) fn enable_service(adb: &Adb, serial: &str) -> Result<(), Failure> {
    enable_service_bounded(adb, serial, BIND_BUDGET)
}

/// [`enable_service`] with an explicit bind-poll deadline.
pub(crate) fn enable_service_bounded(
    adb: &Adb,
    serial: &str,
    budget: Duration,
) -> Result<(), Failure> {
    let raw = settings_get(adb, serial, "enabled_accessibility_services")?;
    let merged = merge_enabled_services(&raw, SERVICE_COMPONENT);
    if merged != raw {
        settings_put(adb, serial, "enabled_accessibility_services", &merged)?;
    }
    if settings_get(adb, serial, "accessibility_enabled")? != "1" {
        settings_put(adb, serial, "accessibility_enabled", "1")?;
    }
    verify_service(adb, serial, budget)
}

/// Reread the setting for an exact component match, then poll `dumpsys
/// accessibility` for the actual bound-service record line.
fn verify_service(adb: &Adb, serial: &str, budget: Duration) -> Result<(), Failure> {
    let reread = settings_get(adb, serial, "enabled_accessibility_services")?;
    if !reread.split(':').any(is_our_service_entry) {
        return Err(bind_failure("service not present after write"));
    }
    let deadline = Instant::now() + budget;
    loop {
        let out = adb.scoped(serial, &["shell", "dumpsys", "accessibility"])?;
        if out
            .stdout
            .lines()
            .any(|l| l.contains("Service[label=Agent Mobile Driver,"))
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(bind_failure("service never bound"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The standard refusal with its manual remedy.
fn bind_failure(why: &str) -> Failure {
    Failure::local(
        format!("accessibility service {why}"),
        "enable it in Settings > Accessibility > Agent Mobile; on Android 13+ \
         first allow App Info > Allow restricted settings",
    )
}

/// `adb -s <serial> install -r <apk>`; an incompatible-signature refusal
/// carries the exact manual remedy and performs no further action.
///
/// # Errors
/// [`Failure::Local`] on any non-success result or `Failure [...]` text.
pub(crate) fn install(adb: &Adb, serial: &str, apk: &Path) -> Result<(), Failure> {
    let path = apk.to_string_lossy().into_owned();
    let out = adb.scoped_with(serial, &["install", "-r", &path], INSTALL_TIMEOUT)?;
    let text = format!("{} {}", out.stdout, out.stderr);
    if out.success && !text.contains("Failure") && !text.contains("Error") {
        return Ok(());
    }
    if text.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
        return Err(Failure::local(
            format!("install refused on {serial}: an incompatible build of {PACKAGE} exists"),
            format!("run `adb -s {serial} uninstall {PACKAGE}` and retry"),
        ));
    }
    Err(Failure::local(
        format!("install failed on {serial}: {}", text.trim()),
        "check the APK and device state, then retry",
    ))
}

/// Locate the debug APK: `AGENT_MOBILE_ANDROID_APK` wins when it names an
/// existing file, else an existing build artifact under `driver_dir`, else
/// invoke the checked-in Gradle wrapper.
///
/// # Errors
/// [`Failure::Local`] when no APK can be produced; never installs SDK,
/// Studio, or other packages.
pub(crate) fn ensure_apk(
    override_apk: Option<&Path>,
    driver_dir: &Path,
    runner: &Arc<dyn CommandRunner>,
) -> Result<PathBuf, Failure> {
    if let Some(apk) = override_apk {
        return if apk.is_file() {
            Ok(apk.to_path_buf())
        } else {
            Err(Failure::local(
                format!("AGENT_MOBILE_ANDROID_APK {} is not a file", apk.display()),
                "point it at an existing driver APK",
            ))
        };
    }
    let apk = driver_dir.join("app/build/outputs/apk/debug/app-debug.apk");
    if apk.is_file() {
        return Ok(apk);
    }
    build_apk(driver_dir, runner)?;
    if apk.is_file() {
        Ok(apk)
    } else {
        Err(Failure::local(
            format!("gradle build produced no {}", apk.display()),
            "run `drivers/android/gradlew -p drivers/android :app:assembleDebug` manually",
        ))
    }
}

/// Run the checked-in wrapper under the 600 s build bound.
fn build_apk(driver_dir: &Path, runner: &Arc<dyn CommandRunner>) -> Result<(), Failure> {
    let wrapper = driver_dir.join("gradlew");
    if !wrapper.is_file() {
        return Err(Failure::local(
            format!("no driver checkout at {}", driver_dir.display()),
            "clone agent-mobile and retry from the repo root",
        ));
    }
    let dir = driver_dir.to_string_lossy().into_owned();
    let out = runner.run(
        &wrapper,
        &["-p", &dir, ":app:assembleDebug", "--no-daemon"],
        BUILD_TIMEOUT,
    )?;
    if out.success {
        Ok(())
    } else {
        Err(failed_op("local", "gradle assembleDebug", &out))
    }
}

/// Authenticated `status` probe through the forwarded URL before the
/// session is declared ready.
///
/// # Errors
/// [`Failure::Local`] on transport failure or a non-ok envelope.
pub(crate) fn probe_status(url: &str, token: &SecretToken) -> Result<(), Failure> {
    let wire = Wire::with_timeout(url, token.as_str(), Duration::from_secs(15));
    let env = wire.call("status", &json!({}))?;
    if env.ok {
        Ok(())
    } else {
        Err(Failure::local(
            "driver status probe returned an error envelope",
            "check `adb logcat` for the driver service",
        ))
    }
}

#[cfg(test)]
mod tests;
