//! Serial HTTP bridge on `127.0.0.1`: every verb except `launch` and
//! `terminate` relays byte-for-byte to the forwarded on-device listener;
//! the two lifecycle verbs run locally against `adb` and call the driver
//! back through [`Wire`] for their snapshot/status halves.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use agent_mobile_core::error::Failure;

use crate::driver::SecretToken;
use crate::lifecycle::LifecycleControl;
use crate::proxy::{
    IO_TIMEOUT, REPLY_CAP, Request, elapsed_ms, error_envelope, find_head_end, http_response,
    read_request,
};
use crate::routes::lifecycle_route;

/// A running bridge: one accept loop, one client at a time, bounded and
/// idempotent [`Bridge::stop`].
pub(crate) struct Bridge {
    port: u16,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Option<TcpStream>>>,
    #[cfg(test)]
    accepted: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>>,
    handle: Option<JoinHandle<()>>,
}

/// Bind `127.0.0.1:0` and start the serial accept loop.
///
/// # Errors
/// [`Failure::Local`] on bind/thread failures.
pub(crate) fn start_bridge(
    upstream_port: u16,
    token: &SecretToken,
    lifecycle: Arc<dyn LifecycleControl>,
) -> Result<Bridge, Failure> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(Failure::from)?;
    let port = listener.local_addr().map_err(Failure::from)?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let active: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
    let accepted = Arc::new(Mutex::new(None));
    let handle = {
        let stop = stop.clone();
        let active = active.clone();
        let accepted = accepted.clone();
        let token = token.clone();
        thread_spawn(move || {
            serve(
                listener,
                upstream_port,
                token,
                lifecycle,
                stop,
                active,
                accepted,
            );
        })?
    };
    Ok(Bridge {
        port,
        stop,
        active,
        #[cfg(test)]
        accepted,
        handle: Some(handle),
    })
}

/// `std::thread::spawn` named so failure paths stay [`Failure`]-shaped.
fn thread_spawn(f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>, Failure> {
    std::thread::Builder::new().spawn(f).map_err(Failure::from)
}

/// Accept loop: serial, one action at a time, exits on `stop`.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the loop body owns these values inside a 'static thread"
)]
fn serve(
    listener: TcpListener,
    upstream_port: u16,
    token: SecretToken,
    lifecycle: Arc<dyn LifecycleControl>,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Option<TcpStream>>>,
    accepted: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>>,
) {
    while !stop.load(Ordering::SeqCst) {
        let Ok((mut sock, _)) = listener.accept() else {
            continue;
        };
        {
            let Ok(mut slot) = active.lock() else {
                break;
            };
            if stop.load(Ordering::SeqCst) {
                let _ = sock.shutdown(Shutdown::Both);
                break;
            }
            *slot = sock.try_clone().ok();
        }
        if let Ok(guard) = accepted.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(());
        }
        handle_client(&mut sock, upstream_port, &token, &lifecycle);
        if let Ok(mut slot) = active.lock() {
            *slot = None;
        }
    }
}

/// Read and dispatch one request; every failure path still writes an
/// envelope-shaped reply where a request was readable.
fn handle_client(
    sock: &mut TcpStream,
    upstream_port: u16,
    token: &SecretToken,
    lifecycle: &Arc<dyn LifecycleControl>,
) {
    let started = Instant::now();
    let _ = sock.set_read_timeout(Some(IO_TIMEOUT));
    let _ = sock.set_write_timeout(Some(IO_TIMEOUT));
    let Ok(req) = read_request(sock) else {
        return;
    };
    let route = req.path.split('?').next().unwrap_or("");
    match route {
        "/launch" | "/terminate" => {
            lifecycle_route(
                sock,
                &req,
                &route[1..],
                upstream_port,
                token,
                lifecycle,
                started,
            );
        }
        _ => relay(sock, &req, upstream_port, started),
    }
}

/// Relay `req.raw` to the forwarded listener and copy the reply back,
/// preserving status and bytes for every envelope upstream sends. Any
/// connect/write/read failure becomes a structured `DRIVER_ERROR` 500 —
/// never an EOF or a partial reply.
fn relay(sock: &mut TcpStream, req: &Request, upstream_port: u16, started: Instant) {
    let verb = req.path.trim_matches('/');
    if let Ok(reply) = relay_once(&req.raw, upstream_port) {
        let _ = sock.write_all(&reply);
    } else {
        let body = error_envelope(
            Some(verb),
            Some(elapsed_ms(started)),
            "DRIVER_ERROR",
            "upstream device listener failed",
        );
        let _ = sock.write_all(&http_response("500 Internal Server Error", &body));
    }
}

/// One upstream round trip: connect, send the raw request, read a strict
/// reply.
fn relay_once(raw: &[u8], upstream_port: u16) -> std::io::Result<Vec<u8>> {
    let mut up = TcpStream::connect(("127.0.0.1", upstream_port))?;
    up.set_read_timeout(Some(IO_TIMEOUT))?;
    up.set_write_timeout(Some(IO_TIMEOUT))?;
    up.write_all(raw)?;
    read_reply(&mut up)
}

/// Validate a reply head ending at `pos` and return the total byte count
/// the full reply must occupy: `HTTP/` status line, no transfer-encoding,
/// exactly one numeric content-length, and the whole reply within
/// [`REPLY_CAP`].
fn parse_reply_head(raw: &[u8], pos: usize) -> std::io::Result<usize> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let head = String::from_utf8_lossy(&raw[..pos]);
    if !head.lines().next().is_some_and(|l| l.starts_with("HTTP/")) {
        return Err(bad("malformed reply head"));
    }
    if head
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:"))
    {
        return Err(bad("reply must not use transfer-encoding"));
    }
    let cls: Vec<&str> = head
        .lines()
        .filter(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .collect();
    if cls.len() != 1 {
        return Err(bad("reply needs exactly one content-length"));
    }
    let len = cls[0]
        .split_once(':')
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .ok_or_else(|| bad("reply content-length is not numeric"))?;
    let total = pos + 4 + len;
    if total > REPLY_CAP {
        return Err(bad("reply exceeds cap"));
    }
    Ok(total)
}

/// Read one upstream reply strictly: `HTTP/` status line, exactly one
/// numeric `Content-Length` not exceeding [`REPLY_CAP`], then precisely
/// that many body bytes — EOF, truncation, and malformation are all errors.
fn read_reply(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    let mut scan_from = 0;
    let mut need: Option<usize> = None;
    loop {
        if raw.len() > REPLY_CAP {
            return Err(bad("reply too large"));
        }
        if need.is_none() {
            if let Some(pos) = find_head_end(&raw, scan_from) {
                need = Some(parse_reply_head(&raw, pos)?);
            } else {
                scan_from = raw.len().saturating_sub(3);
            }
        }
        if let Some(total) = need
            && raw.len() >= total
        {
            raw.truncate(total);
            return Ok(raw);
        }
        match reader.read(&mut buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "reply truncated",
                ));
            }
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => return Err(e),
        }
    }
}

impl Bridge {
    /// Port the bridge bound on `127.0.0.1`.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Whether the accept thread is alive and no stop was requested.
    pub(crate) fn is_running(&self) -> bool {
        !self.stop.load(Ordering::SeqCst) && self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Test hook: signal each accepted client after it is installed in
    /// `active`, so shutdown tests can synchronize without sleeps.
    #[cfg(test)]
    pub(crate) fn notify_on_accept(&self, tx: std::sync::mpsc::Sender<()>) {
        if let Ok(mut guard) = self.accepted.lock() {
            *guard = Some(tx);
        }
    }

    /// Bounded idempotent shutdown: flag the loop, close the in-flight
    /// client, wake the blocked accept, and join the thread.
    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.active.lock()
            && let Some(client) = slot.take()
        {
            let _ = client.shutdown(Shutdown::Both);
        }
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
