//! Process control for serve and lazy start (KTD7, KTD8): supervised
//! children that reap on drop, pid and TCP liveness probes, the atomic
//! boot lockfile, token minting, and log-file-backed child output that
//! doubles as the trust-refusal scan source.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Failure::from)?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().map_err(Failure::from)?.is_some() {
            return child.wait_with_output().map_err(Failure::from);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::local(
                format!("`{program}` did not answer within {}s", timeout.as_secs()),
                "run the probe yourself to see what it is waiting on",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `/bin/kill -<flag> <pid>` with all stdio detached; `true` when the signal
/// was delivered.
fn signal(pid: u32, flag: &str) -> bool {
    Command::new("/bin/kill")
        .args([flag, &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Is `pid` a live process? Uses `kill -0`, which needs no permission for
/// our own children and answers "no" for zombies and reused-pid misses.
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    signal(pid, "-0")
}

/// SIGTERM `pid` — the graceful stop for a child that owns its own cleanup
/// (`serve` forwards it to the runner and clears session state first).
/// Returns whether the signal was delivered.
#[must_use]
pub fn terminate(pid: u32) -> bool {
    signal(pid, "-TERM")
}

/// Send SIGTERM to `pid` only when `ps` still shows it as an xcodebuild —
/// the comm check keeps a recycled pid safe, and the target is always a pid
/// we recorded ourselves (KTD17). Returns whether the signal was sent.
#[must_use]
pub fn terminate_runner(pid: u32) -> bool {
    let is_runner = Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("xcodebuild"))
        .unwrap_or(false);
    is_runner && signal(pid, "-TERM")
}

/// Poll until `pid` dies or `budget` expires; returns true when it exited.
#[must_use]
pub fn await_exit(pid: u32, budget: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < budget {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !pid_alive(pid)
}

/// Does `host:port` accept a TCP connection? Hostnames resolve first, so
/// `Lahfirs-iPhone.local:8770` works the same as a literal. One probe per
/// call; callers that poll should resolve once and use [`tcp_ready_at`].
#[must_use]
pub fn tcp_ready(addr: &str) -> bool {
    let Ok(addrs) = addr.to_socket_addrs() else {
        return false;
    };
    tcp_ready_at(&addrs.collect::<Vec<_>>())
}

/// Probe a pre-resolved address set; [`tcp_ready`] minus the per-poll DNS
/// lookup — mDNS resolution of a `.local` name is the expensive part.
#[must_use]
pub fn tcp_ready_at(addrs: &[SocketAddr]) -> bool {
    addrs
        .iter()
        .any(|a| TcpStream::connect_timeout(a, Duration::from_millis(500)).is_ok())
}

/// Atomic boot lockfile (KTD7): `create_new` either wins or reports the
/// holder; the file disappears when the holder finishes. Stale locks older
/// than `stale_after` may be reclaimed by the caller.
pub struct BootLock {
    path: PathBuf,
}

impl BootLock {
    /// Try to take the boot lock at `path`.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] on filesystem errors other than
    /// already-exists.
    pub fn take(path: &Path) -> Result<Option<Self>, Failure> {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(_) => Ok(Some(Self {
                path: path.to_path_buf(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(e) => Err(Failure::from(e)),
        }
    }

    /// Age of an existing lockfile; `None` when absent or unreadable.
    #[must_use]
    pub fn age(path: &Path) -> Option<Duration> {
        let meta = fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        Some(modified.elapsed().unwrap_or(Duration::MAX))
    }

    /// Remove the lockfile at `path` (stale reclaim).
    pub fn clear(path: &Path) {
        let _ = fs::remove_file(path);
    }
}

impl Drop for BootLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
