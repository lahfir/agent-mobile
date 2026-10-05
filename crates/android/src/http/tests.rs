//! Relay byte-preservation and bounded shutdown proofs.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;

use super::{accept_retryable, read_reply, start_bridge, start_bridge_with_deadline};
use crate::driver::SecretToken;
use crate::testkit::{ctl, envelope, fake_upstream, post, read_http_request};

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

#[test]
fn reply_reads_when_terminator_splits_and_truncates_trail() -> std::io::Result<()> {
    use std::io::Read as _;
    let head_a: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r";
    let head_b: &[u8] = b"\nDATAPADDED-EXTRA";
    let mut reader = head_a.chain(head_b);
    let reply = read_reply(&mut reader)?;
    assert_eq!(reply, b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nDATA");
    Ok(())
}

#[test]
fn accept_retryable_only_for_interrupted() {
    assert!(accept_retryable(&std::io::Error::from(
        std::io::ErrorKind::Interrupted
    )));
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::ConnectionAborted,
        std::io::ErrorKind::AddrInUse,
    ] {
        assert!(!accept_retryable(&std::io::Error::from(kind)));
    }
}

#[test]
fn trickle_bytes_lose_to_absolute_deadline() -> Result<(), Failure> {
    let (up, _rx) = fake_upstream(vec![("/snapshot".into(), envelope("200 OK", "{}"))]);
    let mut bridge = start_bridge_with_deadline(
        up,
        &SecretToken::new("t0k"),
        ctl(),
        Duration::from_millis(400),
    )?;
    let mut c = TcpStream::connect(("127.0.0.1", bridge.port()))?;
    c.set_read_timeout(Some(Duration::from_secs(2)))?;
    c.write_all(b"POST /sn").map_err(Failure::from)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        let _ = c.write_all(b"x");
        let mark = start.elapsed() + Duration::from_millis(50);
        while start.elapsed() < mark {
            std::hint::spin_loop();
        }
    }
    let mut buf = [0u8; 8];
    let closed = !matches!(c.read(&mut buf), Ok(n) if n > 0);
    assert!(closed, "deadline-surviving connection still open");
    let env = post(
        bridge.port(),
        "POST /snapshot HTTP/1.1\r\nAuthorization: Bearer t0k\r\nHost: b\r\n",
        "",
    );
    assert!(env.starts_with("HTTP/1.1 200"), "{env}");
    bridge.stop();
    Ok(())
}

#[test]
fn second_request_while_busy_gets_503_without_reaching_upstream() {
    use std::io::Write;
    let (seen_tx, seen_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| unreachable!());
    let upstream_port = listener
        .local_addr()
        .unwrap_or_else(|_| unreachable!())
        .port();
    std::thread::spawn(move || {
        let mut first = true;
        while let Ok((mut sock, _)) = listener.accept() {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let _req = read_http_request(&mut sock);
            let _ = seen_tx.send(());
            if first {
                first = false;
                let _ = release_rx.recv();
            }
            let body = "{\"version\":\"1\",\"ok\":true,\"command\":\"status\",\"elapsed_ms\":1,\"data\":{\"app\":\"x\",\"snapshot_id\":\"\",\"device\":\"d\",\"os\":\"1\"}}";
            let reply = envelope("200 OK", body);
            let _ = sock.write_all(reply.as_bytes());
            let _ = sock.shutdown(std::net::Shutdown::Both);
        }
    });

    let mut bridge = start_bridge(upstream_port, &SecretToken::new("t0k"), ctl())
        .unwrap_or_else(|e| unreachable!("{}", e.render()));
    let port = bridge.port();
    let head =
        "POST /status HTTP/1.1\r\nAuthorization: Bearer t0k\r\nX-Agent-Mobile-Version: 1\r\n";
    let first = std::thread::spawn(move || post(port, head, "{}"));
    seen_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| unreachable!("first request never reached upstream"));

    let second = post(
        port,
        "POST /status HTTP/1.1\r\nAuthorization: Bearer t0k\r\nX-Agent-Mobile-Version: 1\r\n",
        "{}",
    );
    assert!(second.contains("503 Service Unavailable"), "{second}");
    assert!(second.contains("DRIVER_ERROR"), "{second}");
    assert!(
        second.contains("another command is in progress"),
        "{second}"
    );
    assert!(
        seen_rx.try_recv().is_err(),
        "a queued request must never reach upstream"
    );

    let _ = release_tx.send(());
    let out = first.join().unwrap_or_default();
    assert!(out.contains("HTTP/1.1 200"), "{out}");
    let third = post(
        port,
        "POST /status HTTP/1.1\r\nAuthorization: Bearer t0k\r\nX-Agent-Mobile-Version: 1\r\n",
        "{}",
    );
    assert!(third.contains("HTTP/1.1 200"), "{third}");
    assert_eq!(
        seen_rx.try_iter().count(),
        1,
        "third request reached upstream once"
    );
    bridge.stop();
}
