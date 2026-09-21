//! Shared helpers for the core crate's test binaries — each test file is its
//! own crate, so an item a given binary never calls carries a reasoned
//! allowance instead of tripping `dead_code`.

use agent_mobile_core::error::Failure;

/// Test-failure constructor shared across the core test binaries.
pub fn fail(msg: &str) -> Failure {
    Failure::local(msg.to_owned(), "fix the test")
}

/// Is `buf` a complete HTTP/1 request — headers plus a full
/// `Content-Length` body?
#[allow(
    dead_code,
    reason = "only the wire tests stub the driver over a socket"
)]
pub fn request_complete(buf: &[u8]) -> bool {
    let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    buf.len() >= pos + 4 + len
}
