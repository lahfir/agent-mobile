//! Relay byte-preservation and bounded shutdown proofs.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::time::Duration;

use agent_mobile_core::error::Failure;

use super::start_bridge;
use crate::driver::SecretToken;
use crate::testkit::{ctl, envelope, fake_upstream, post};

#[test]
fn relay_preserves_request_and_response_bytes() -> Result<(), Failure> {
    for status in [
        "200 OK",
        "401 Unauthorized",
        "409 Conflict",
        "500 Internal Server Error",
    ] {
        let reply = envelope(status, "{\"version\":\"1\",\"ok\":true}");
        let (up, rx) = fake_upstream(vec![("/snapshot".into(), reply)]);
        let mut bridge = start_bridge(up, &SecretToken::new("t0k"), ctl())?;
        let body = "{\"app\":\"x\"}";
        let raw = format!(
            "POST /snapshot HTTP/1.1\r\nHost: b\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut sock = TcpStream::connect(("127.0.0.1", bridge.port())).map_err(Failure::from)?;
        sock.write_all(raw.as_bytes()).map_err(Failure::from)?;
        let upstream_got = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| Failure::local("upstream never saw request", "fail"))?;
        assert_eq!(String::from_utf8_lossy(&upstream_got), raw);
        let mut out = Vec::new();
        sock.set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(Failure::from)?;
        sock.read_to_end(&mut out).map_err(Failure::from)?;
        assert!(String::from_utf8_lossy(&out).starts_with(&format!("HTTP/1.1 {status}")));
        assert!(String::from_utf8_lossy(&out).ends_with("\"ok\":true}"));
        bridge.stop();
    }
    Ok(())
}

#[test]
fn dead_upstream_becomes_driver_error_envelope() -> Result<(), Failure> {
    let mut bridge = start_bridge(1, &SecretToken::new("t0k"), ctl())?;
    let out = post(
        bridge.port(),
        "POST /status HTTP/1.1\r\nAuthorization: Bearer t0k\r\nX-Agent-Mobile-Version: 1\r\n",
        "{}",
    );
    assert!(out.contains("DRIVER_ERROR"));
    assert!(out.starts_with("HTTP/1.1 500"));
    bridge.stop();
    Ok(())
}

#[test]
fn stop_closes_listener_and_stalled_client() -> Result<(), Failure> {
    let mut bridge = start_bridge(1, &SecretToken::new("t0k"), ctl())?;
    let (tx, rx) = channel();
    bridge.notify_on_accept(tx);
    let mut stalled = TcpStream::connect(("127.0.0.1", bridge.port())).map_err(Failure::from)?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| Failure::local("client never became active", "fail"))?;
    stalled
        .write_all(b"POST /x HTTP/1.1\r\nHost: h\r\n")
        .map_err(Failure::from)?;
    let t = std::time::Instant::now();
    bridge.stop();
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "stop waited on stalled client"
    );
    assert!(TcpStream::connect(("127.0.0.1", bridge.port())).is_err());
    Ok(())
}

#[test]
fn bridge_reports_running_then_stopped() -> Result<(), Failure> {
    let mut bridge = start_bridge(1, &SecretToken::new("t0k"), ctl())?;
    assert!(bridge.is_running());
    bridge.stop();
    assert!(!bridge.is_running());
    Ok(())
}
