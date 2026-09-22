//! Stub-server tests for the wire client: envelopes survive every status,
//! all three headers go out, and transport failures stay structured.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;
use std::time::Duration;

use agent_mobile_core::contract::Data;
use agent_mobile_core::error::{ErrorCode, Failure};
use agent_mobile_core::wire::Wire;

mod common;
use common::{fail, request_complete};

fn stub_once(status: u16, body: &str) -> Result<(String, JoinHandle<String>), Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let payload = body.to_owned();
    let join = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return String::new();
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        while !request_complete(&buf) {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let resp = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        String::from_utf8_lossy(&buf).into_owned()
    });
    Ok((format!("http://127.0.0.1:{port}"), join))
}

fn stub_hang(hold: Duration) -> Result<(String, JoinHandle<String>), Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let join = std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return String::new();
        };
        std::thread::sleep(hold);
        drop(stream);
        String::new()
    });
    Ok((format!("http://127.0.0.1:{port}"), join))
}

fn refused_url() -> Result<String, Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(format!("http://127.0.0.1:{port}"))
}

const SNAPSHOT: &str = r#"{"version":"1","ok":true,"command":"snapshot","elapsed_ms":42,"data":{"app":"com.apple.springboard","snapshot_id":"abc","ref_count":1,"complete":true,"settled":true,"reads":2,"text":"line","tree":{"role":"application","name":"SpringBoard","value":"","ref_id":"@abc:e1","states":[],"available_actions":[],"bounds":{"x":0.0,"y":0.0,"width":430.0,"height":930.0},"children":[]}}}"#;

#[test]
fn stale_ref_409_parses_code_and_exits_1() -> Result<(), Failure> {
    let body = r#"{"version":"1","ok":false,"command":"tap","elapsed_ms":4,"error":{"code":"STALE_REF","message":"ref gone"}}"#;
    let (base, join) = stub_once(409, body)?;
    let wire = Wire::new(&base, "tok");
    let reply = wire.call("tap", &serde_json::json!({"ref":"@x:e1"}))?;
    let _ = join.join();
    let envelope = reply;
    assert!(!envelope.ok);
    let err = envelope.error.ok_or_else(|| fail("missing error body"))?;
    assert_eq!(err.code, "STALE_REF");
    let f = Failure::from_error_body(&err);
    assert_eq!(f.exit_code(), 1);
    assert!(f.render().contains("STALE_REF"));
    assert!(f.render().contains("re-snapshot"));
    Ok(())
}

#[test]
fn unauthorized_401_omits_command_and_hints_token() -> Result<(), Failure> {
    let body =
        r#"{"version":"1","ok":false,"error":{"code":"UNAUTHORIZED","message":"bad token"}}"#;
    let (base, join) = stub_once(401, body)?;
    let wire = Wire::new(&base, "wrong");
    let reply = wire.call("status", &serde_json::json!({}))?;
    let _ = join.join();
    let envelope = reply;
    assert!(envelope.command.is_none());
    assert!(envelope.elapsed_ms.is_none());
    let err = envelope.error.ok_or_else(|| fail("missing error body"))?;
    let f = Failure::from_error_body(&err);
    assert!(f.render().contains("UNAUTHORIZED"));
    assert!(f.render().contains("AGENT_MOBILE_TOKEN"));
    Ok(())
}

#[test]
fn refused_connection_synthesizes_transport_error() -> Result<(), Failure> {
    let base = refused_url()?;
    let wire = Wire::new(&base, "tok");
    match wire.call("status", &serde_json::json!({})) {
        Err(Failure::Transport { .. }) => {}
        Err(other) => return Err(fail(&format!("expected Transport, got {}", other.render()))),
        Ok(_) => return Err(fail("refused connection must not succeed")),
    }
    let rendered = match wire.call("status", &serde_json::json!({})) {
        Err(f) => f.render(),
        Ok(_) => return Err(fail("refused connection must not succeed")),
    };
    assert!(rendered.contains("DRIVER_ERROR"));
    assert!(rendered.contains("unreachable"));
    assert!(rendered.contains("human-only"));
    Ok(())
}

#[test]
fn envelope_version_mismatch_fails_fast() -> Result<(), Failure> {
    let body = r#"{"version":"2","ok":true,"command":"status","elapsed_ms":1,"data":{"app":"a","snapshot_id":"","device":"d","os":"1"}}"#;
    let (base, join) = stub_once(200, body)?;
    let wire = Wire::new(&base, "tok");
    let result = wire.call("status", &serde_json::json!({}));
    let _ = join.join();
    match result {
        Err(Failure::Local { message, .. }) => {
            assert!(message.contains("\"2\""), "{message}");
            assert!(message.contains("version mismatch"), "{message}");
        }
        Err(other) => {
            return Err(fail(&format!(
                "expected a local failure, got {}",
                other.render()
            )));
        }
        Ok(_) => return Err(fail("version mismatch must fail")),
    }
    Ok(())
}

#[test]
fn request_carries_all_three_headers() -> Result<(), Failure> {
    let body = r#"{"version":"1","ok":true,"command":"status","elapsed_ms":1,"data":{"app":"a","snapshot_id":"","device":"d","os":"1"}}"#;
    let (base, join) = stub_once(200, body)?;
    let wire = Wire::new(&base, "t0k3n");
    let _ = wire.call("status", &serde_json::json!({}));
    let captured = match join.join() {
        Ok(c) => c.to_lowercase(),
        Err(_) => return Err(fail("stub thread panicked")),
    };
    assert!(captured.contains("post /status "), "{captured}");
    assert!(
        captured.contains("authorization: bearer t0k3n"),
        "{captured}"
    );
    assert!(captured.contains("connection: close"), "{captured}");
    assert!(captured.contains("x-agent-mobile-version: 1"), "{captured}");
    assert!(
        captured.contains("content-type: application/json"),
        "{captured}"
    );
    Ok(())
}

#[test]
fn hung_driver_trips_timeout_as_transport() -> Result<(), Failure> {
    let (base, join) = stub_hang(Duration::from_secs(2))?;
    let wire = Wire::with_timeout(&base, "tok", Duration::from_millis(300));
    match wire.call("status", &serde_json::json!({})) {
        Err(Failure::Transport { .. }) => {}
        Err(other) => return Err(fail(&format!("expected Transport, got {}", other.render()))),
        Ok(_) => return Err(fail("hung driver must not succeed")),
    }
    let _ = join.join();
    Ok(())
}

#[test]
fn snapshot_envelope_parses_typed_data() -> Result<(), Failure> {
    let (base, join) = stub_once(200, SNAPSHOT)?;
    let wire = Wire::new(&base, "tok");
    let reply = wire.call("snapshot", &serde_json::json!({}))?;
    let _ = join.join();
    assert!(reply.ok);
    let Data::Snapshot(snap) = reply.data.ok_or_else(|| fail("missing data"))? else {
        return Err(fail("expected Snapshot data"));
    };
    assert_eq!(snap.snapshot_id, "abc");
    assert!(snap.settled);
    assert_eq!(snap.reads, 2);
    assert_eq!(snap.tree.ref_id, "@abc:e1");
    Ok(())
}

#[test]
fn malformed_body_is_driver_error() -> Result<(), Failure> {
    let (base, join) = stub_once(200, "not json at all")?;
    let wire = Wire::new(&base, "tok");
    let result = wire.call("status", &serde_json::json!({}));
    let _ = join.join();
    match result {
        Err(Failure::Driver {
            code: ErrorCode::DriverError,
            ..
        }) => {}
        Err(other) => {
            return Err(fail(&format!(
                "expected DriverError, got {}",
                other.render()
            )));
        }
        Ok(_) => return Err(fail("malformed body must not succeed")),
    }
    Ok(())
}

#[test]
fn driver_returned_driver_error_retries_once() -> Result<(), Failure> {
    let body = r#"{"version":"1","ok":false,"command":"tap","elapsed_ms":1,"error":{"code":"DRIVER_ERROR","message":"boom"}}"#;
    let (base, join) = stub_once(500, body)?;
    let wire = Wire::new(&base, "tok");
    let reply = wire.call("tap", &serde_json::json!({"x":1.0,"y":2.0}))?;
    let _ = join.join();
    let err = reply.error.ok_or_else(|| fail("missing error body"))?;
    let rendered = Failure::from_error_body(&err).render();
    assert!(rendered.contains("retry once"));
    assert!(
        !rendered.contains("unreachable"),
        "driver-returned errors must not print the transport stanza"
    );
    Ok(())
}

#[test]
fn unknown_command_parses_code() -> Result<(), Failure> {
    let body = r#"{"version":"1","ok":false,"command":"bogus","elapsed_ms":0,"error":{"code":"UNKNOWN_COMMAND","message":"bogus"}}"#;
    let (base, join) = stub_once(409, body)?;
    let wire = Wire::new(&base, "tok");
    let reply = wire.call("bogus", &serde_json::json!({}))?;
    let _ = join.join();
    let err = reply.error.ok_or_else(|| fail("missing error body"))?;
    assert_eq!(err.code, "UNKNOWN_COMMAND");
    Ok(())
}
