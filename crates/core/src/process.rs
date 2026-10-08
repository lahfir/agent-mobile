//! Process control for serve and lazy start (KTD7, KTD8): supervised
//! children that reap on drop, pid and TCP liveness probes, the atomic
//! boot lockfile, token minting, and log-file-backed child output that
//! doubles as the trust-refusal scan source.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

use crate::error::Failure;

/// Token alphabet: 24 lowercase hex chars from 12 random bytes.
const TOKEN_BYTES: usize = 12;

/// Whole-boot ceiling shared by `serve` and lazy start; a cold xcodebuild
/// plus a simulator boot can take minutes.
pub const BOOT_BUDGET_DEFAULT: Duration = Duration::from_secs(240);
/// Env var raising the boot ceiling on a slow machine, in whole seconds.
pub const BOOT_BUDGET_ENV: &str = "AGENT_MOBILE_BOOT_BUDGET_SECS";

/// The boot ceiling for this invocation: the default, unless
/// [`BOOT_BUDGET_ENV`] names a larger whole number of seconds.
#[must_use]
pub fn boot_budget() -> Duration {
    budget_from(std::env::var(BOOT_BUDGET_ENV).ok().as_deref())
}

/// [`boot_budget`] with the env value passed in, so it is testable.
fn budget_from(raw: Option<&str>) -> Duration {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map_or(BOOT_BUDGET_DEFAULT, Duration::from_secs)
}

/// Poll interval for boot waits in `serve` and lazy start.
pub const BOOT_POLL: Duration = Duration::from_millis(500);

/// Mint a bearer token from `/dev/urandom`; never a default, never logged.
///
/// # Errors
/// Returns [`Failure::Local`] when the random source cannot be read.
pub fn mint_token() -> Result<String, Failure> {
    let mut f = File::open("/dev/urandom")?;
    let mut bytes = [0u8; TOKEN_BYTES];
    f.read_exact(&mut bytes)?;
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for b in bytes {
        write!(out, "{b:02x}").map_err(|e| {
            Failure::local(format!("token encode failed: {e}"), "retry the command")
        })?;
    }
    Ok(out)
}

/// A child process whose stdout and stderr stream into one log file;
/// [`ServeChild::new_output`] replays what arrived since the last call so a
/// supervisor can scan for known failure text without piping.
pub struct ServeChild {
    child: Child,
    log: PathBuf,
    offset: u64,
}

impl ServeChild {
    /// Spawn `cmd` with stdio redirected to `log`; the file is created
    /// append-mode so a second serve keeps the first run's output. The
    /// output cursor starts at the file's current length — history written
    /// by earlier runs must never replay into this one's marker scans.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the log cannot be opened or the child
    /// cannot be spawned.
    pub fn spawn_logged(cmd: &mut Command, log: &Path) -> Result<Self, Failure> {
        let file = open_private_log(log)?;
        let err_file = file.try_clone()?;
        let offset = file.metadata().map(|m| m.len()).unwrap_or(0);
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::from(file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .map_err(|e| {
                Failure::local(
                    format!("cannot spawn {}: {e}", cmd.get_program().to_string_lossy()),
                    "fix the local problem and retry",
                )
            })?;
        Ok(Self {
            child,
            log: log.to_path_buf(),
            offset,
        })
    }

    /// The child's pid.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Bytes appended to the log since the last call.
    pub fn new_output(&mut self) -> String {
        let Ok(mut f) = File::open(&self.log) else {
            return String::new();
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return String::new();
        }
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_err() {
            return String::new();
        }
        self.offset += buf.len() as u64;
        buf
    }

    /// Exit status once the child has finished; `None` while running.
    ///
    /// # Errors
    /// Returns the `wait` io error.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, Failure> {
        self.child.try_wait().map_err(Failure::from)
    }

    /// Block until the child exits.
    ///
    /// # Errors
    /// Returns the `wait` io error.
    pub fn wait(&mut self) -> Result<std::process::ExitStatus, Failure> {
        self.child.wait().map_err(Failure::from)
    }

    /// Signal the owned child to die — only ever this spawned pid (KTD17).
    ///
    /// # Errors
    /// Returns the `kill` io error.
    pub fn kill(&mut self) -> Result<(), Failure> {
        self.child.kill().map_err(Failure::from)
    }
}

/// Open `path` append-mode at `0o600`, creating private parents first —
/// the recipe every child log uses, shared by `serve` and lazy start.
///
/// # Errors
/// Returns [`Failure::Local`] when the dirs cannot be created or the file
/// cannot be opened.
pub fn open_private_log(path: &Path) -> Result<File, Failure> {
    if let Some(parent) = path.parent() {
        crate::secret::create_private_dirs(parent)?;
    }
    Ok(OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?)
}

impl Drop for ServeChild {
    /// KTD8 drop guard: a serve that exits for any reason kills and reaps
    /// its runner, so nothing holds the port after we're gone.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `cmd` with piped output and a deadline; a child that outlives
/// `timeout` is killed and reported, so a wedged probe (`xcrun simctl`,
/// `devicectl`) cannot hang a verb forever.
///
/// # Errors
/// Returns [`Failure::Local`] on spawn, wait, or timeout failures.
pub fn run_bounded(cmd: &mut Command, timeout: Duration) -> Result<std::process::Output, Failure> {
    let never = std::sync::atomic::AtomicBool::new(false);
    run_bounded_until(cmd, timeout, &never)
}

/// [`run_bounded`] that also watches `cancelled`: a set flag kills and
/// reaps the child and reports interruption rather than the timeout
/// message. Cancellation never suppresses the wait — a killed child is
/// always reaped before returning.
///
/// The child runs in its own process group so cancellation/timeout kills
/// the whole tree, and both pipes drain on reader threads so output can
/// never fill a pipe while the parent waits for exit. Reader threads are
/// joined on every path so no descendant-held pipe is left behind.
///
/// # Errors
/// Spawn/io failures, timeout, or `cancelled` interruption.
pub fn run_bounded_until(
    cmd: &mut Command,
    timeout: Duration,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<std::process::Output, Failure> {
    use std::os::unix::process::CommandExt as _;
    let program = cmd.get_program().to_string_lossy().into_owned();
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(Failure::local("operation interrupted", "rerun the command"));
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(Failure::from)?;
    let stdout_reader = drain(child.stdout.take());
    let stderr_reader = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(_)) => break Done::Exited,
            Ok(None) => {}
            Err(e) => break Done::Io(Failure::from(e)),
        }
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            break Done::Interrupted;
        }
        if Instant::now() > deadline {
            break Done::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !matches!(outcome, Done::Exited) {
        kill_group(&child);
        let _ = child.kill();
    }
    let wait_status = child.wait();
    kill_group(&child);
    let join = |h: std::thread::JoinHandle<std::io::Result<Captured>>| {
        h.join()
            .map_err(|_| Failure::local("output reader thread panicked", "rerun the command"))?
            .map_err(Failure::from)
    };
    let stdout = join(stdout_reader);
    let stderr = join(stderr_reader);
    let status = match wait_status {
        Ok(s) => s,
        Err(e) => {
            return Err(Failure::local(
                format!("wait failed: {e}"),
                "rerun the command",
            ));
        }
    };
    match outcome {
        Done::Io(e) => return Err(e),
        Done::Interrupted => {
            return Err(Failure::local("operation interrupted", "rerun the command"));
        }
        Done::TimedOut => {
            return Err(Failure::local(
                format!("`{program}` did not answer within {}s", timeout.as_secs()),
                "run the probe yourself to see what it is waiting on",
            ));
        }
        Done::Exited => {}
    }
    let stdout = stdout?;
    let stderr = stderr?;
    if stdout.exceeded || stderr.exceeded {
        let which = match (stdout.exceeded, stderr.exceeded) {
            (true, true) => "stdout and stderr",
            (true, false) => "stdout",
            (false, true) => "stderr",
            (false, false) => "no stream",
        };
        return Err(Failure::local(
            format!("`{program}` {which} exceeded the 8 MiB capture limit"),
            "reduce the command's output or run it yourself",
        ));
    }
    let stdout = stdout.bytes;
    let stderr = stderr.bytes;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Which way `run_bounded_until` left its poll loop.
enum Done {
    Exited,
    Interrupted,
    TimedOut,
    Io(Failure),
}

/// Drain one child pipe to a Vec on its own thread.
/// Hard cap on retained bytes per stream — output past this is drained
/// and discarded so the child never blocks, but never returned as if it
/// were complete.
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// One stream's capture: retained bytes plus the overrun flag.
struct Captured {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn drain(
    pipe: Option<impl Read + Send + 'static>,
) -> std::thread::JoinHandle<std::io::Result<Captured>> {
    std::thread::spawn(move || {
        let mut cap = Captured {
            bytes: Vec::new(),
            exceeded: false,
        };
        let Some(mut p) = pipe else {
            return Ok(cap);
        };
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match p.read(&mut chunk) {
                Ok(0) => return Ok(cap),
                Ok(n) => {
                    let take = (MAX_CAPTURE_BYTES - cap.bytes.len()).min(n);
                    cap.bytes.extend_from_slice(&chunk[..take]);
                    cap.exceeded = cap.exceeded || take < n;
                }
                Err(e) => return Err(e),
            }
        }
    })
}

/// SIGKILL the owned child's process group — best effort; the
/// direct-child `Child::kill` fallback is handled by the caller.
fn kill_group(child: &Child) {
    let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
}

mod lifecycle;
pub use lifecycle::{
    BootLock, await_exit, pid_alive, process_identity, process_matches, tcp_ready, tcp_ready_at,
    terminate, terminate_runner,
};

#[cfg(test)]
mod budget_tests {
    use super::{BOOT_BUDGET_DEFAULT, budget_from};
    use std::time::Duration;

    #[test]
    fn unset_or_junk_keeps_the_default() {
        for raw in [
            None,
            Some(
                "
",
            ),
            Some("soon"),
            Some("0"),
            Some("-5"),
        ] {
            assert_eq!(budget_from(raw), BOOT_BUDGET_DEFAULT);
        }
    }

    #[test]
    fn a_whole_number_of_seconds_wins() {
        assert_eq!(budget_from(Some(" 600 ")), Duration::from_secs(600));
    }
}

#[cfg(test)]
mod tests;
