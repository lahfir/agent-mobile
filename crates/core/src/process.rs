//! Process control for serve and lazy start (KTD7, KTD8): supervised
//! children that reap on drop, pid and TCP liveness probes, the atomic
//! boot lockfile, token minting, and log-file-backed child output that
//! doubles as the trust-refusal scan source.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::Failure;

/// Token alphabet: 24 lowercase hex chars from 12 random bytes.
const TOKEN_BYTES: usize = 12;

/// Whole-boot ceiling shared by `serve` and lazy start; a cold xcodebuild
/// plus a simulator boot can take minutes.
pub const BOOT_BUDGET: Duration = Duration::from_secs(240);
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
    /// append-mode so a second serve keeps the first run's output.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the log cannot be opened or the child
    /// cannot be spawned.
    pub fn spawn_logged(cmd: &mut Command, log: &Path) -> Result<Self, Failure> {
        if let Some(parent) = log.parent() {
            crate::secret::create_private_dirs(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(log)?;
        let err_file = file.try_clone()?;
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
            offset: 0,
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

impl Drop for ServeChild {
    /// KTD8 drop guard: a serve that exits for any reason kills and reaps
    /// its runner, so nothing holds the port after we're gone.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Is `pid` a live process? Uses `kill -0`, which needs no permission for
/// our own children and answers "no" for zombies and reused-pid misses.
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Send SIGTERM to `pid` only when `ps` still shows it as an xcodebuild —
/// the comm check keeps a recycled pid safe, and the target is always a pid
/// we recorded ourselves (KTD17). Returns whether the signal was sent.
#[must_use]
pub fn terminate_runner(pid: u32) -> bool {
    let is_runner = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("xcodebuild"))
        .unwrap_or(false);
    is_runner
        && Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
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
/// call; callers loop with their own deadline.
#[must_use]
pub fn tcp_ready(addr: &str) -> bool {
    let Ok(addrs) = addr.to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(500)).is_ok())
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
