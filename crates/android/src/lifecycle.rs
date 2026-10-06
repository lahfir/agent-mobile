//! `launch`/`terminate` over serial-scoped `adb`: `am`/`cmd package`
//! invocations only, conservative package validation, and a terminate
//! refusal list that can never drop the launcher, system UI, or driver.

use std::fmt;
use std::time::Duration;

use crate::adb::{Adb, DEFAULT_TIMEOUT, diagnostic_output};
use crate::driver::PACKAGE;

/// Launcher category for `resolve-activity`.
const CAT_LAUNCHER: &str = "android.intent.category.LAUNCHER";
/// HOME category resolves the user's current launcher package.
const CAT_HOME: &str = "android.intent.category.HOME";

/// `am start -W` blocks until the activity's first frame — a cold app
/// start legitimately outlives the default command deadline.
const APP_START_TIMEOUT: Duration = Duration::from_secs(60);

/// Lifecycle failure split: caller-shaped mistakes vs device failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    /// The request itself is invalid — mirrors `BAD_REQUEST`.
    BadRequest(String),
    /// The platform refused or failed — mirrors `DRIVER_ERROR`.
    Driver(String),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(m) | Self::Driver(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for LifecycleError {}

/// App lifecycle surface the HTTP bridge calls. Production runs serial-
/// scoped `adb`; tests inject fakes.
pub trait LifecycleControl: Send + Sync {
    /// Cold-start `package` at its launcher activity.
    ///
    /// # Errors
    /// [`LifecycleError::BadRequest`] for invalid/unresolvable packages,
    /// [`LifecycleError::Driver`] for device-side failures.
    fn launch(&self, package: &str) -> Result<(), LifecycleError>;

    /// Force-stop `package` — the bridge supplies only the app it observed
    /// upstream; protected packages refuse.
    ///
    /// # Errors
    /// [`LifecycleError::BadRequest`] for protected/invalid packages,
    /// [`LifecycleError::Driver`] for device-side failures.
    fn terminate(&self, package: &str) -> Result<(), LifecycleError>;
}

/// `adb`-backed lifecycle for one serial.
pub(crate) struct AdbLifecycle {
    adb: Adb,
    serial: String,
}

/// `package` must be dot-separated ASCII Java identifiers.
pub(crate) fn valid_package(package: &str) -> bool {
    !package.is_empty()
        && package.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// A resolved `pkg/class` is usable only with exactly one slash, a valid
/// package, and a nonempty class of `[A-Za-z0-9_.$]`; an explicit package
/// lookup must resolve to that same package — anything else fails before
/// any `force-stop`/`start` mutation reaches the device.
fn valid_component(component: &str, package: Option<&str>) -> Result<String, LifecycleError> {
    let bad = || LifecycleError::BadRequest(format!("malformed component {component:?}"));
    if component.matches('/').count() != 1 {
        return Err(bad());
    }
    let (pkg, class) = component.split_once('/').ok_or_else(bad)?;
    if !valid_package(pkg)
        || class.is_empty()
        || !class
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'))
    {
        return Err(bad());
    }
    if package.is_some_and(|p| p != pkg) {
        return Err(LifecycleError::BadRequest(format!(
            "package {package:?} resolved to foreign component {component:?}"
        )));
    }
    Ok(component.to_owned())
}

impl AdbLifecycle {
    /// Control bound to one serial.
    #[must_use]
    pub(crate) fn new(adb: Adb, serial: impl Into<String>) -> Self {
        Self {
            adb,
            serial: serial.into(),
        }
    }

    /// One remote shell command (words quoted at the adb boundary) that
    /// must succeed and carry no `Error:`/`Exception` text.
    fn shell(&self, op: &str, args: &[&str]) -> Result<String, LifecycleError> {
        self.shell_with(op, args, DEFAULT_TIMEOUT)
    }

    /// [`shell`] under an explicit per-call deadline chosen by the
    /// caller — `shell` delegates with [`DEFAULT_TIMEOUT`].
    fn shell_with(
        &self,
        op: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, LifecycleError> {
        let out = self
            .adb
            .remote_shell_with(&self.serial, args, timeout)
            .map_err(|e| LifecycleError::Driver(format!("adb {op} failed: {}", e.message())))?;
        let text = format!("{} {}", out.stdout, out.stderr);
        let bad_line = text.lines().any(|line| {
            let t = line.trim_start();
            t.starts_with("Error:") || t.starts_with("Exception")
        });
        if !out.success || bad_line {
            return Err(LifecycleError::Driver(format!(
                "adb {op} failed on {}: {}",
                self.serial,
                diagnostic_output(&out)
            )));
        }
        Ok(out.stdout)
    }

    /// `cmd package resolve-activity --brief` → exactly one `pkg/class`.
    /// `package` is only present for explicit per-package lookups.
    fn resolve(&self, package: Option<&str>, category: &str) -> Result<String, LifecycleError> {
        let mut args = vec![
            "cmd",
            "package",
            "resolve-activity",
            "--brief",
            "--user",
            "current",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            category,
        ];
        if let Some(pkg) = package {
            args.push(pkg);
        }
        let out = self.shell("resolve-activity", &args)?;
        let components: Vec<&str> = out
            .lines()
            .map(str::trim)
            .filter(|l| l.contains('/') && !l.contains(' '))
            .collect();
        match components.as_slice() {
            [single] => Ok(valid_component(single, package)?),
            _ => Err(LifecycleError::BadRequest(format!(
                "resolve-activity returned {} components",
                components.len()
            ))),
        }
    }

    /// The package the HOME intent currently resolves to.
    fn launcher_package(&self) -> Result<String, LifecycleError> {
        let component = self.resolve(None, CAT_HOME)?;
        component
            .split('/')
            .next()
            .map(str::to_owned)
            .ok_or_else(|| LifecycleError::Driver("launcher resolve returned no package".into()))
    }
}

impl LifecycleControl for AdbLifecycle {
    fn launch(&self, package: &str) -> Result<(), LifecycleError> {
        if !valid_package(package) {
            return Err(LifecycleError::BadRequest(format!(
                "invalid package name {package:?}"
            )));
        }
        let component = self.resolve(Some(package), CAT_LAUNCHER)?;
        self.shell("force-stop", &["am", "force-stop", package])?;
        self.shell_with(
            "start",
            &["am", "start", "-W", "-n", &component],
            APP_START_TIMEOUT,
        )?;
        Ok(())
    }

    fn terminate(&self, package: &str) -> Result<(), LifecycleError> {
        if !valid_package(package) {
            return Err(LifecycleError::BadRequest(format!(
                "invalid package name {package:?}"
            )));
        }
        for protected in ["android", PACKAGE, "com.android.systemui"] {
            if package == protected {
                return Err(LifecycleError::BadRequest(format!(
                    "refusing to terminate protected package {package}"
                )));
            }
        }
        if self.launcher_package()? == package {
            return Err(LifecycleError::BadRequest(format!(
                "refusing to terminate the home launcher {package}"
            )));
        }
        self.shell("force-stop", &["am", "force-stop", package])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
