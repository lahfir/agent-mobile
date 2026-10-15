//! Process-lifecycle helpers: signal/liveness/identity markers, bounded
//! termination, TCP readiness, and the atomic boot lockfile.

use std::fs::{self, OpenOptions};

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::Failure;

/// `/bin/kill -<flag> <pid>` with all stdio detached; `true` when the signal
/// was delivered.
pub(crate) fn signal(pid: u32, flag: &str) -> bool {
    Command::new("/bin/kill")
        .args([flag, &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Is `pid` present in the process table? Uses `kill -0`, which needs no
/// permission for our own children. An absent pid is `false`, but a
/// *reused* pid is present/`true` and a zombie still counts — this is
/// liveness only, never identity (use [`process_identity`] /
/// [`process_matches`] for that).
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    signal(pid, "-0")
}

/// A marker identifying this exact process incarnation — Linux reads
/// `/proc/<pid>/stat` field 22 (starttime jiffies) after the final `)`;
/// other Unix uses `ps -o lstart= -p <pid>`. The marker is prefixed with
/// its source so the two forms can never be confused across machines.
///
/// `None` when the pid is dead or unreadable.
#[must_use]
pub fn process_identity(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let close = stat.rfind(')')?;
        let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        let starttime = fields.get(19)?;
        Some(format!("proc:{starttime}"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = Command::new("/bin/ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (!text.is_empty()).then(|| format!("ps:{text}"))
    }
}

/// Is `pid` the same process incarnation `expected` recorded? Only an
/// exact recorded birth marker establishes identity — markerless legacy
/// rows (`expected` = `None`) remain loadable but are stale: they must be
/// restarted before any bearer token is read or sent, so `None` never
/// authorizes. A recycled pid can never impersonate the recorded owner.
#[must_use]
pub fn process_matches(pid: u32, expected: Option<&str>) -> bool {
    expected.is_some_and(|marker| process_identity(pid).is_some_and(|m| m == marker))
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
