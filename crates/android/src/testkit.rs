//! Shared fakes: a scripted [`CommandRunner`] that records every argv and
//! replays canned outputs, so unit tests prove exact command shapes without
//! ever touching a device.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_mobile_core::error::Failure;

use crate::adb::{CommandOutput, CommandRunner};
use crate::lifecycle::{LifecycleControl, LifecycleError};

/// One canned reply: a status flag plus cleaned stdout/stderr text.
#[must_use]
pub fn output(success: bool, stdout: &str, stderr: &str) -> CommandOutput {
    CommandOutput {
        success,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

/// Runner that replays `replies` in order (success-empty when exhausted)
/// while recording every `(program, argv)` it was asked to run.
pub(crate) struct FakeRunner {
    calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    replies: Mutex<VecDeque<CommandOutput>>,
}

impl FakeRunner {
    /// Shared fake with canned replies.
    #[must_use]
    pub(crate) fn scripted(replies: Vec<CommandOutput>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            replies: Mutex::new(replies.into()),
        })
    }

    /// Every argv observed so far.
    pub(crate) fn calls(&self) -> Vec<Vec<String>> {
        self.calls
            .lock()
            .map(|c| c.iter().map(|(_, a)| a.clone()).collect())
            .unwrap_or_default()
    }
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        _timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((
                program.to_path_buf(),
                args.iter().map(ToString::to_string).collect(),
            ));
        }
        let mut replies = self
            .replies
            .lock()
            .map_err(|_| Failure::local("fake runner lock poisoned", "rerun the test"))?;
        Ok(replies.pop_front().unwrap_or_else(|| output(true, "", "")))
    }
}

/// Total bytes `head + 4 + content-length` when the head terminates, else
/// `None`.
pub(crate) fn head_len(got: &[u8]) -> Option<usize> {
    let pos = got.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&got[..pos]);
    let len = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| {
            l.split_once(':')
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    Some(pos + 4 + len)
}

/// Read one request off a socket, tolerating head/body segment splits.
pub(crate) fn read_http_request(sock: &mut std::net::TcpStream) -> Vec<u8> {
    use std::io::Read;
    let mut got = Vec::new();
    let mut buf = [0u8; 8192];
    let mut stalls = 0;
    loop {
        if head_len(&got).is_some_and(|want| got.len() >= want) {
            break;
        }
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                stalls = 0;
                got.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                stalls += 1;
                if stalls > 2 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    got
}

/// Recording lifecycle fake for bridge tests.
pub(crate) struct FakeCtl {
    /// Packages passed to `launch`.
    pub(crate) launched: Mutex<Vec<String>>,
    /// Packages passed to `terminate`.
    pub(crate) terminated: Mutex<Vec<String>>,
}

impl LifecycleControl for FakeCtl {
    fn launch(&self, package: &str) -> Result<(), LifecycleError> {
        self.launched
            .lock()
            .map_err(|_| LifecycleError::Driver("lock".into()))?
            .push(package.to_owned());
        Ok(())
    }

    fn terminate(&self, package: &str) -> Result<(), LifecycleError> {
        self.terminated
            .lock()
            .map_err(|_| LifecycleError::Driver("lock".into()))?
            .push(package.to_owned());
        Ok(())
    }
}

/// Empty recording fake.
#[must_use]
pub(crate) fn ctl() -> Arc<FakeCtl> {
    Arc::new(FakeCtl {
        launched: Mutex::new(vec![]),
        terminated: Mutex::new(vec![]),
    })
}

/// Raw-reply upstream: `answers` maps request path → verbatim response;
/// the special reply `CLOSE` drops the socket without writing.
#[must_use]
pub(crate) fn fake_upstream(
    answers: Vec<(String, String)>,
) -> (u16, std::sync::mpsc::Receiver<Vec<u8>>) {
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc::channel;
    use std::thread;
    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
        return (0, channel().1);
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let (tx, rx) = channel();
    thread::spawn(move || {
        let mut remaining = answers;
        while let Ok((mut sock, _)) = listener.accept() {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let got = read_http_request(&mut sock);
            let _ = tx.send(got.clone());
            let path = String::from_utf8_lossy(&got)
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .to_owned();
            let reply = remaining.iter().position(|(p, _)| path == *p).map_or_else(
                || "HTTP/1.1 500 X\r\nContent-Length: 2\r\n\r\n{}".to_owned(),
                |i| remaining.remove(i).1,
            );
            if reply == "CLOSE" {
                drop(sock);
                continue;
            }
            let _ = sock.write_all(reply.as_bytes());
            let _ = sock.shutdown(std::net::Shutdown::Both);
        }
    });
    (port, rx)
}

/// `status` + `body` into a full HTTP reply string.
#[must_use]
pub(crate) fn envelope(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// POST `head` + `body` to `port`, returning the raw reply text.
#[must_use]
pub(crate) fn post(port: u16, head: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let Ok(mut sock) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
    let req = format!("{head}Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = sock.write_all(req.as_bytes());
    let mut out = Vec::new();
    let _ = sock.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}
