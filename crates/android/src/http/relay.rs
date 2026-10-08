//! Upstream relay for non-lifecycle requests: connect to the forwarded
//! on-device listener, send the raw request bytes, read a strict reply.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use crate::proxy::{
    IO_TIMEOUT, REPLY_CAP, Request, UPSTREAM_TIMEOUT, elapsed_ms, error_envelope, find_head_end,
    http_response,
};

/// Relay `req.raw` to the forwarded listener and copy the reply back,
/// preserving status and bytes for every envelope upstream sends. Any
/// connect/write/read failure becomes a structured `DRIVER_ERROR` 500 —
/// never an EOF or a partial reply. Every relay resets
/// `upstream_failures` on success and saturates it upward on failure, so
/// [`Bridge::is_running`](super::Bridge::is_running) can trip the session
/// when the device side dies behind a live accept loop.
pub(super) fn relay(
    sock: &mut TcpStream,
    req: &Request,
    upstream_port: u16,
    started: Instant,
    upstream_failures: &AtomicU32,
) {
    let verb = req.path.trim_matches('/');
    if let Ok(reply) = relay_once(&req.raw, upstream_port) {
        upstream_failures.store(0, Ordering::SeqCst);
        let _ = sock.write_all(&reply);
    } else {
        let _ = upstream_failures.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            Some(n.saturating_add(1))
        });
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
    let addr = SocketAddr::from(([127, 0, 0, 1], upstream_port));
    let mut up = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    up.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
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
    let total = pos
        .checked_add(4)
        .and_then(|head| head.checked_add(len))
        .ok_or_else(|| bad("reply size overflows"))?;
    if total > REPLY_CAP {
        return Err(bad("reply exceeds cap"));
    }
    Ok(total)
}

/// Read one upstream reply strictly: `HTTP/` status line, exactly one
/// numeric `Content-Length` not exceeding [`REPLY_CAP`], then precisely
/// that many body bytes — EOF, truncation, and malformation are all errors.
pub(super) fn read_reply(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
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
