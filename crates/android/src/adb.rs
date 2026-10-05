//! Bounded tool seam: every `adb` device operation prefixes `-s <serial>`,
//! output is owned and control-cleaned, and an injectable [`CommandRunner`]
//! lets tests script exact argv and replies without touching a device.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use agent_mobile_core::error::Failure;
use agent_mobile_core::process::run_bounded;

/// Default deadline for one `adb` op — a wedged transport dies in 15 s.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// `install -r` gets a longer budget: APK push over a cold channel is slow.
pub(crate) const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Gradle builds get the longest pole.
pub(crate) const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

/// Bytes of child output kept for error text and parsing.
const OUTPUT_CAP: usize = 4 * 1024;

/// Owned result of one bounded run.
///
/// Debug deliberately reports only lengths and status — stdout/stderr can
/// carry provision output and must never appear in logs or errors.
#[derive(Clone)]
pub(crate) struct CommandOutput {
    /// Exit status was zero.
    pub(crate) success: bool,
    /// stdout with control bytes stripped.
    pub(crate) stdout: String,
    /// stderr with control bytes stripped.
    pub(crate) stderr: String,
}

impl std::fmt::Debug for CommandOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandOutput")
            .field("success", &self.success)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish()
    }
}

/// Strip ASCII control bytes (except newline/tab) and cap the length, so
/// device text can never smuggle escape sequences into errors or logs.
pub(crate) fn clean_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len().min(OUTPUT_CAP));
    for c in text.chars() {
        if out.len() >= OUTPUT_CAP {
            break;
        }
        if !c.is_control() || c == '\n' || c == '\t' {
            out.push(c);
        }
    }
    out
}

/// How one program run executes; the real runner uses [`run_bounded`],
/// tests script canned replies and record argv.
pub(crate) trait CommandRunner: Send + Sync {
    /// Run `program` with `args` under `timeout`.
    ///
    /// # Errors
    /// Spawn, io, or deadline failures.
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure>;
}

struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        let out = run_bounded(&mut cmd, timeout).map_err(|e| {
            Failure::local(
                format!("{}: {}", program.display(), e.message()),
                "check the tool installation and retry",
            )
        })?;
        Ok(CommandOutput {
            success: out.status.success(),
            stdout: clean_output(&out.stdout),
            stderr: clean_output(&out.stderr),
        })
    }
}

/// SDK binary locations after environment/platform resolution.
pub(crate) struct SdkTools {
    /// `adb` executable path.
    pub(crate) adb: PathBuf,
    /// `emulator` executable path.
    pub(crate) emulator: PathBuf,
}

/// Platform default SDK directory under `home`.
fn platform_default_sdk(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Android/sdk")
    } else {
        home.join("Android/Sdk")
    }
}

/// Resolution order: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, the platform
/// default SDK path, then bare PATH names so a missing SDK reports through
/// the bounded probe rather than a silent lookup.
#[must_use]
pub(crate) fn resolve_sdk(
    get: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> SdkTools {
    let mut roots = Vec::new();
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = get(key)
            && !root.is_empty()
        {
            roots.push(PathBuf::from(root));
        }
    }
    if let Some(home) = std::env::home_dir() {
        roots.push(platform_default_sdk(&home));
    }
    for root in roots {
        let adb = root.join("platform-tools").join("adb");
        if exists(&adb) {
            return SdkTools {
                adb,
                emulator: root.join("emulator").join("emulator"),
            };
        }
    }
    SdkTools {
        adb: PathBuf::from("adb"),
        emulator: PathBuf::from("emulator"),
    }
}

/// Serial-scoped `adb` facade; every device command prefixes `-s <serial>`
/// so two sessions can never cross targets.
#[derive(Clone)]
pub(crate) struct Adb {
    exe: PathBuf,
    runner: Arc<dyn CommandRunner>,
}

impl Adb {
    /// Real-process runner over `exe`.
    #[must_use]
    pub(crate) fn real(exe: PathBuf) -> Self {
        Self::with_runner(exe, Arc::new(RealRunner))
    }

    /// Injected-runner constructor for tests.
    #[must_use]
    pub(crate) fn with_runner(exe: PathBuf, runner: Arc<dyn CommandRunner>) -> Self {
        Self { exe, runner }
    }

    /// The backing runner — lets callers drive sibling tools (`emulator`,
    /// `gradlew`) through the same injectable seam.
    pub(crate) fn runner(&self) -> &Arc<dyn CommandRunner> {
        &self.runner
    }

    /// Run a non-`adb` tool (`emulator -list-avds`) under the default bound.
    pub(crate) fn tool(&self, program: &Path, args: &[&str]) -> Result<CommandOutput, Failure> {
        self.runner.run(program, args, DEFAULT_TIMEOUT)
    }

    /// `adb <args>` with no serial — only for host-level commands like
    /// `devices -l` and `version`.
    pub(crate) fn unscoped(&self, args: &[&str]) -> Result<CommandOutput, Failure> {
        self.runner.run(&self.exe, args, DEFAULT_TIMEOUT)
    }

    /// `adb -s <serial> <args>` under the default deadline.
    pub(crate) fn scoped(&self, serial: &str, args: &[&str]) -> Result<CommandOutput, Failure> {
        self.scoped_with(serial, args, DEFAULT_TIMEOUT)
    }

    /// `adb -s <serial> <args>` under an explicit deadline.
    pub(crate) fn scoped_with(
        &self,
        serial: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        let mut full = Vec::with_capacity(args.len() + 2);
        full.push("-s");
        full.push(serial);
        full.extend_from_slice(args);
        self.runner.run(&self.exe, &full, timeout)
    }

    /// [`scoped`] that also requires a zero exit, folding stderr into a
    /// named-operation failure.
    pub(crate) fn scoped_ok(
        &self,
        serial: &str,
        op: &str,
        args: &[&str],
    ) -> Result<CommandOutput, Failure> {
        let out = self.scoped(serial, args)?;
        if out.success {
            Ok(out)
        } else {
            Err(failed_op(serial, op, &out))
        }
    }
}

/// One `adb` op failure carrying program, serial, and trimmed output.
pub(crate) fn failed_op(serial: &str, op: &str, out: &CommandOutput) -> Failure {
    let detail = format!("{} {}", out.stderr, out.stdout).trim().to_owned();
    Failure::local(
        format!("adb {op} failed on {serial}: {detail}"),
        "inspect the device state and retry",
    )
}

#[cfg(test)]
mod tests;
