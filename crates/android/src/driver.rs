//! On-device driver setup: locate or build the debug APK, install it,
//! provision a bearer token that lives only in memory, merge-enable the
//! accessibility service without disturbing existing entries, and create a
//! verified owned `adb forward`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_mobile_core::contract::{Data, Envelope};
use agent_mobile_core::error::Failure;
use agent_mobile_core::wire::Wire;
use serde_json::json;

use crate::adb::{Adb, BUILD_TIMEOUT, INSTALL_TIMEOUT, diagnostic_output, failed_op};

/// Driver application id.
pub const PACKAGE: &str = "com.lahfir.agentmobile.driver";
/// Accessibility service component as `settings secure` stores it.
pub const SERVICE_COMPONENT: &str =
    "com.lahfir.agentmobile.driver/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService";
/// Content-provider authority that mints session tokens.
pub const PROVIDER_URI: &str = "content://com.lahfir.agentmobile.driver.provision";
/// Device-side listener port on pre-dynamic state rows.
pub const LEGACY_DEVICE_PORT: u16 = 8770;

/// Token length minted by the provider: 32 bytes base64url, no padding.
const TOKEN_LEN: usize = 43;

/// How long to poll `dumpsys accessibility` for the service bind.
const BIND_BUDGET: Duration = Duration::from_secs(20);

/// One `provision` result: the bearer token plus its bound listener port.
pub(crate) struct Provisioned {
    pub(crate) token: SecretToken,
    pub(crate) device_port: u16,
}

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

/// `content call --uri PROVIDER_URI --method provision` returns token +
/// bound listener port; neither token nor raw output touches argv, env,
/// logs, or error text.
///
/// # Errors
/// [`Failure::Local`] on refusal or unusable token/port.
pub(crate) fn provision(adb: &Adb, serial: &str) -> Result<Provisioned, Failure> {
    let out = adb.remote_shell(
        serial,
        &[
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
    let Some(token) = extract_token(&out.stdout) else {
        return Err(Failure::local(
            "provision returned no token",
            "check the driver service is installed and retry",
        ));
    };
    let Some(device_port) = extract_port(&out.stdout) else {
        return Err(Failure::local(
            "provision returned no valid port",
            "check the driver service is installed and retry",
        ));
    };
    Ok(Provisioned {
        token: SecretToken(token),
        device_port,
    })
}

/// The single complete `port=<decimal>` field, 1..=65535.
fn extract_port(output: &str) -> Option<u16> {
    let bytes = output.as_bytes();
    let mut found: Option<&str> = None;
    let mut pos = 0;
    while let Some(rel) = output[pos..].find("port=") {
        let start = pos + rel;
        pos = start + "port=".len();
        if start > 0 && !is_boundary(bytes[start - 1]) {
            continue;
        }
        let end = bytes[pos..]
            .iter()
            .position(|b| !b.is_ascii_digit())
            .map_or(bytes.len(), |r| pos + r);
        let complete = end > pos
            && (end == bytes.len()
                || matches!(bytes[end], b'}' | b']' | b',' | b' ' | b'\t' | b'\n'));
        if !complete || found.is_some() {
            return None;
        }
        found = Some(&output[pos..end]);
        pos = end;
    }
    found?.parse::<u16>().ok().filter(|p| *p > 0)
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
    let detail = diagnostic_output(&out);
    if text.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
        return Err(Failure::local(
            format!("install refused on {serial}: an incompatible build of {PACKAGE} exists"),
            format!("run `adb -s {serial} uninstall {PACKAGE}` and retry"),
        ));
    }
    Err(Failure::local(
        format!("install failed on {serial}: {detail}"),
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
    adb: &Adb,
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
    build_apk(driver_dir, adb)?;
    if apk.is_file() {
        Ok(apk)
    } else {
        Err(Failure::local(
            format!("gradle build produced no {}", apk.display()),
            "run `drivers/android/gradlew -p drivers/android :app:assembleDebug` manually",
        ))
    }
}

/// Run the checked-in wrapper under the 600 s build bound — through
/// [`Adb::tool_with`] so a cancellation flag reaches the Gradle child.
fn build_apk(driver_dir: &Path, adb: &Adb) -> Result<(), Failure> {
    let wrapper = driver_dir.join("gradlew");
    if !wrapper.is_file() {
        return Err(Failure::local(
            format!("no driver checkout at {}", driver_dir.display()),
            "clone agent-mobile and retry from the repo root",
        ));
    }
    let dir = driver_dir.to_string_lossy().into_owned();
    let out = adb.tool_with(
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
/// [`Failure::Local`] on transport failure, a non-ok envelope, or a
/// well-formed envelope that is not an authenticated `status` payload.
pub(crate) fn probe_status(url: &str, token: &SecretToken) -> Result<(), Failure> {
    let wire = Wire::with_timeout(url, token.as_str(), Duration::from_secs(3));
    let env = wire.call("status", &json!({}))?;
    if is_status_envelope(&env) {
        Ok(())
    } else {
        Err(Failure::local(
            "driver status probe returned an error envelope",
            "check `adb logcat` for the driver service",
        ))
    }
}

pub(crate) fn is_status_envelope(env: &Envelope) -> bool {
    env.ok && env.command.as_deref() == Some("status") && matches!(env.data, Some(Data::Status(_)))
}

mod enable;
pub(crate) use enable::enable_service;
#[cfg(test)]
pub(crate) use enable::{enable_service_bounded, merge_enabled_services};
#[cfg(test)]
mod tests;
