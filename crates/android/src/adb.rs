//! Bounded tool seam: every `adb` device operation prefixes `-s <serial>`,
//! output is owned and control-cleaned, and an injectable [`CommandRunner`]
//! lets tests script exact argv and replies without touching a device.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use std::sync::atomic::{AtomicBool, Ordering};

use agent_mobile_core::error::Failure;
use agent_mobile_core::process::run_bounded_until;

/// Default deadline for one `adb` op — a wedged transport dies in 15 s.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// `install -r` gets a longer budget: APK push over a cold channel is slow.
pub(crate) const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Gradle builds get the longest pole.
pub(crate) const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

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

/// Strip ASCII control bytes (except newline/tab) so device text can
/// never smuggle escape sequences into errors or logs — machine parsers
/// always see the complete output; only diagnostics are capped.
pub(crate) fn clean_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// Byte cap for failure diagnostics — complete stdout/stderr stay on
/// [`CommandOutput`]; only rendered errors are shortened.
const DIAGNOSTIC_CAP: usize = 4 * 1024;

/// Char-safe cap at [`DIAGNOSTIC_CAP`] plus an explicit truncation marker.
pub(crate) fn bounded_diagnostic(text: &str) -> String {
    if text.len() <= DIAGNOSTIC_CAP {
        return text.to_owned();
    }
    let mut end = DIAGNOSTIC_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…[{} bytes truncated]", &text[..end], text.len() - end)
}

/// How one program run executes; the real runner uses
/// [`run_bounded_until`], tests script canned replies and record argv.
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

    /// [`run`] that also honours a cancellation flag: checked before
    /// spawning and polled while the child runs.
    ///
    /// # Errors
    /// Same as [`run`], plus interruption when `cancelled` is set.
    fn run_until(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<CommandOutput, Failure> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Failure::local("operation interrupted", "rerun the command"));
        }
        self.run(program, args, timeout)
    }
}

struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        self.run_until(program, args, timeout, &AtomicBool::new(false))
    }

    fn run_until(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<CommandOutput, Failure> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Failure::local("operation interrupted", "rerun the command"));
        }
        let mut cmd = Command::new(program);
        cmd.args(args);
        let out = run_bounded_until(&mut cmd, timeout, cancelled).map_err(|e| {
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
    cancelled: Option<Arc<AtomicBool>>,
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
        Self {
            exe,
            runner,
            cancelled: None,
        }
    }

    /// Clone carrying a cancellation flag — every runner call polls it.
    #[must_use]
    pub(crate) fn with_cancellation(&self, flag: Arc<AtomicBool>) -> Self {
        Self {
            exe: self.exe.clone(),
            runner: self.runner.clone(),
            cancelled: Some(flag),
        }
    }

    /// Whether this clone's cancellation flag is currently set.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
    }

    /// Clone with no cancellation flag — cleanup paths must never see a
    /// request to stop mid-teardown.
    #[must_use]
    pub(crate) fn without_cancellation(&self) -> Self {
        Self {
            exe: self.exe.clone(),
            runner: self.runner.clone(),
            cancelled: None,
        }
    }

    /// Dispatch through `run_until` when a flag is present, else `run`.
    fn call(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        match &self.cancelled {
            Some(flag) => self.runner.run_until(program, args, timeout, flag),
            None => self.runner.run(program, args, timeout),
        }
    }

    /// Run a non-`adb` tool (`emulator -list-avds`, `gradlew`) under the
    /// default bound — through [`Adb::call`] so cancellation reaches it.
    pub(crate) fn tool(&self, program: &Path, args: &[&str]) -> Result<CommandOutput, Failure> {
        self.call(program, args, DEFAULT_TIMEOUT)
    }

    /// A non-`adb` tool with an explicit deadline (Gradle assembly).
    pub(crate) fn tool_with(
        &self,
        program: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        self.call(program, args, timeout)
    }

    /// `adb <args>` with no serial — only for host-level commands like
    /// `devices -l` and `version`.
    pub(crate) fn unscoped(&self, args: &[&str]) -> Result<CommandOutput, Failure> {
        self.call(&self.exe, args, DEFAULT_TIMEOUT)
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
        self.call(&self.exe, &full, timeout)
    }

    /// `adb -s <serial> shell <one quoted command string>`: every remote
    /// word is single-quote escaped at this canonical boundary.
    pub(crate) fn remote_shell(
        &self,
        serial: &str,
        args: &[&str],
    ) -> Result<CommandOutput, Failure> {
        self.remote_shell_with(serial, args, DEFAULT_TIMEOUT)
    }

    /// [`remote_shell`] under an explicit deadline — same canonical word
    /// quoting; use for operations that legitimately outlive the default.
    pub(crate) fn remote_shell_with(
        &self,
        serial: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        let command = args
            .iter()
            .map(|arg| quote_remote_word(arg))
            .collect::<Vec<_>>()
            .join(" ");
        self.scoped_with(serial, &["shell", &command], timeout)
    }

    /// [`remote_shell`] that also requires a zero exit, folding stderr
    /// into a named-operation failure.
    pub(crate) fn remote_shell_ok(
        &self,
        serial: &str,
        op: &str,
        args: &[&str],
    ) -> Result<CommandOutput, Failure> {
        let out = self.remote_shell(serial, args)?;
        if out.success {
            Ok(out)
        } else {
            Err(failed_op(serial, op, &out))
        }
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

/// POSIX single-quote one remote-shell word — the only escaping rule for
/// arguments handed to `adb shell`.
fn quote_remote_word(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

/// Combined stderr+stdout rendered through [`bounded_diagnostic`] — the
/// only sanctioned way to put command output into human-facing text.
pub(crate) fn diagnostic_output(out: &CommandOutput) -> String {
    bounded_diagnostic(format!("{} {}", out.stderr, out.stdout).trim())
}

/// One `adb` op failure carrying program, serial, and trimmed output.
pub(crate) fn failed_op(serial: &str, op: &str, out: &CommandOutput) -> Failure {
    let detail = diagnostic_output(out);
    Failure::local(
        format!("adb {op} failed on {serial}: {detail}"),
        "inspect the device state and retry",
    )
}

#[cfg(test)]
mod tests;
