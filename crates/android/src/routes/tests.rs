//! Lifecycle-route behavior: auth/version/body gating, upstream calls,
//! command rewriting, and status mapping.

use std::time::Duration;

use agent_mobile_core::error::Failure;

use crate::driver::SecretToken;
use crate::http::start_bridge;
use crate::testkit::{ctl, envelope, fake_upstream, post};

const HEAD: &str = "Authorization: Bearer t0k\r\nX-Agent-Mobile-Version: 1\r\n";

#[test]
fn launch_requires_auth_version_and_body() -> Result<(), Failure> {
    let snap = envelope(
        "200 OK",
        "{\"version\":\"1\",\"ok\":true,\"command\":\"snapshot\",\"data\":{\"app\":\"com.pkg\",\"snapshot_id\":\"a1b2c3d4\",\"ref_count\":1,\"complete\":true,\"settled\":true,\"reads\":1,\"text\":\"\",\"tree\":{\"role\":\"g\",\"name\":\"\",\"value\":\"\",\"ref_id\":\"@a1b2c3d4:e1\",\"states\":[],\"available_actions\":[],\"bounds\":{\"x\":0.0,\"y\":0.0,\"width\":1.0,\"height\":1.0},\"children\":[]}}}",
    );
    let (up, _rx) = fake_upstream(vec![("/snapshot".into(), snap)]);
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), ctl())?;
    let bad = post(
        bridge.port(),
        "POST /launch HTTP/1.1\r\nAuthorization: Bearer wrong\r\nX-Agent-Mobile-Version: 1\r\n",
        "{\"bundle_id\":\"com.pkg\"}",
    );
    assert!(bad.starts_with("HTTP/1.1 401"));
    assert!(bad.contains("UNAUTHORIZED"));
    assert!(!bad.contains("\"command\""));
    let nover = post(
        bridge.port(),
        "POST /launch HTTP/1.1\r\nAuthorization: Bearer t0k\r\n",
        "{\"bundle_id\":\"com.pkg\"}",
    );
    assert!(nover.starts_with("HTTP/1.1 409"));
    let badid = post(
        bridge.port(),
        &format!("POST /launch HTTP/1.1\r\n{HEAD}"),
        "{\"bundle_id\":\"a;b\"}",
    );
    assert!(badid.starts_with("HTTP/1.1 409"));
    assert!(badid.contains("BAD_REQUEST"));
    bridge.stop();
    Ok(())
}

#[test]
fn launch_calls_control_then_snapshot_rewrite() -> Result<(), Failure> {
    let snap = envelope(
        "200 OK",
        "{\"version\":\"1\",\"ok\":true,\"command\":\"snapshot\",\"elapsed_ms\":5,\"data\":{\"app\":\"com.pkg\",\"snapshot_id\":\"a1b2c3d4\",\"ref_count\":1,\"complete\":true,\"settled\":true,\"reads\":1,\"text\":\"\",\"tree\":{\"role\":\"g\",\"name\":\"\",\"value\":\"\",\"ref_id\":\"@a1b2c3d4:e1\",\"states\":[],\"available_actions\":[],\"bounds\":{\"x\":0.0,\"y\":0.0,\"width\":1.0,\"height\":1.0},\"children\":[]}}}",
    );
    let (up, rx) = fake_upstream(vec![("/snapshot".into(), snap)]);
    let fake = ctl();
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), fake.clone())?;
    let out = post(
        bridge.port(),
        &format!("POST /launch HTTP/1.1\r\n{HEAD}"),
        "{\"bundle_id\":\"com.pkg\"}",
    );
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.contains("\"command\":\"launch\""));
    assert!(out.contains("\"app\":\"com.pkg\""));
    assert_eq!(
        fake.launched.lock().map(|l| l.clone()).unwrap_or_default(),
        ["com.pkg"]
    );
    let upstream = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| Failure::local("no upstream call", "fail"))?;
    let text = String::from_utf8_lossy(&upstream).into_owned();
    assert!(text.contains("POST /snapshot"), "upstream got: {text}");
    assert!(
        text.contains("\"app\": \"com.pkg\""),
        "upstream got: {text}"
    );
    bridge.stop();
    Ok(())
}

#[test]
fn terminate_uses_observed_app_only() -> Result<(), Failure> {
    let stat = envelope(
        "200 OK",
        "{\"version\":\"1\",\"ok\":true,\"command\":\"status\",\"data\":{\"app\":\"com.observed\",\"snapshot_id\":\"\",\"device\":\"x\",\"os\":\"1\"}}",
    );
    let (up, _rx) = fake_upstream(vec![("/status".into(), stat)]);
    let fake = ctl();
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), fake.clone())?;
    let out = post(
        bridge.port(),
        &format!("POST /terminate HTTP/1.1\r\n{HEAD}"),
        "{\"bundle_id\":\"com.attacker\"}",
    );
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.contains("\"terminated\":\"com.observed\""));
    assert!(out.contains("\"command\":\"terminate\""));
    assert_eq!(
        fake.terminated
            .lock()
            .map(|l| l.clone())
            .unwrap_or_default(),
        ["com.observed"]
    );
    bridge.stop();
    Ok(())
}

#[test]
fn upstream_error_rewrites_command_to_terminate() -> Result<(), Failure> {
    let bad = envelope(
        "409 Conflict",
        "{\"version\":\"1\",\"ok\":false,\"command\":\"status\",\"elapsed_ms\":2,\"error\":{\"code\":\"DRIVER_ERROR\",\"message\":\"no window\"}}",
    );
    let (up, _rx) = fake_upstream(vec![("/status".into(), bad)]);
    let fake = ctl();
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), fake.clone())?;
    let out = post(
        bridge.port(),
        &format!("POST /terminate HTTP/1.1\r\n{HEAD}"),
        "{}",
    );
    assert!(out.contains("\"command\":\"terminate\""));
    assert!(out.contains("no window"));
    assert!(fake.terminated.lock().map(|l| l.is_empty()).unwrap_or(true));
    bridge.stop();
    Ok(())
}

#[test]
fn alias_paths_relay_upstream() -> Result<(), Failure> {
    let (up, _rx) = fake_upstream(vec![(
        "//launch/".into(),
        "HTTP/1.1 418 Teapot\r\nContent-Length: 15\r\n\r\n{\"routed\":true}".to_owned(),
    )]);
    let fake = ctl();
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), fake.clone())?;
    let out = post(
        bridge.port(),
        &format!("POST //launch/ HTTP/1.1\r\n{HEAD}"),
        "{}",
    );
    assert!(out.starts_with("HTTP/1.1 418"), "{out}");
    assert!(out.contains("{\"routed\":true}"));
    assert!(fake.launched.lock().map(|l| l.is_empty()).unwrap_or(true));
    bridge.stop();
    Ok(())
}

#[test]
fn upstream_unauthorized_maps_to_401() -> Result<(), Failure> {
    let snap = envelope(
        "401 Unauthorized",
        "{\"version\":\"1\",\"ok\":false,\"command\":\"snapshot\",\"error\":{\"code\":\"UNAUTHORIZED\",\"message\":\"auth\"}}",
    );
    let (up, _rx) = fake_upstream(vec![("/snapshot".into(), snap)]);
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), ctl())?;
    let out = post(
        bridge.port(),
        &format!("POST /launch HTTP/1.1\r\n{HEAD}"),
        "{\"bundle_id\":\"com.pkg\"}",
    );
    assert!(out.starts_with("HTTP/1.1 401"), "{out}");
    assert!(out.contains("\"command\":\"launch\""));
    bridge.stop();
    Ok(())
}

#[test]
fn upstream_driver_error_maps_to_409() -> Result<(), Failure> {
    let stat = envelope(
        "500 Internal Server Error",
        "{\"version\":\"1\",\"ok\":false,\"command\":\"status\",\"error\":{\"code\":\"DRIVER_ERROR\",\"message\":\"window gone\"}}",
    );
    let (up, _rx) = fake_upstream(vec![("/status".into(), stat)]);
    let mut bridge = start_bridge(up, &SecretToken::new("t0k"), ctl())?;
    let out = post(
        bridge.port(),
        &format!("POST /terminate HTTP/1.1\r\n{HEAD}"),
        "{}",
    );
    assert!(out.starts_with("HTTP/1.1 409"), "{out}");
    assert!(out.contains("\"command\":\"terminate\""));
    bridge.stop();
    Ok(())
}

/// Relay a `status` POST through a bridge whose upstream answers `reply`.
fn relayed_status(reply: &str) -> String {
    let (up, _rx) = fake_upstream(vec![("/status".into(), reply.to_owned())]);
    let Ok(mut bridge) = start_bridge(up, &SecretToken::new("t0k"), ctl()) else {
        return String::new();
    };
    let out = post(
        bridge.port(),
        &format!("POST /status HTTP/1.1\r\n{HEAD}"),
        "{}",
    );
    bridge.stop();
    out
}

#[test]
fn upstream_closing_without_reply_is_driver_error() {
    let out = relayed_status("CLOSE");
    assert!(out.starts_with("HTTP/1.1 500"), "{out}");
    assert!(out.contains("DRIVER_ERROR"), "{out}");
}

#[test]
fn truncated_upstream_reply_is_driver_error() {
    let out = relayed_status("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort");
    assert!(out.starts_with("HTTP/1.1 500"), "{out}");
    assert!(out.contains("DRIVER_ERROR"), "{out}");
}

#[test]
fn oversized_upstream_reply_is_driver_error() {
    let out = relayed_status("HTTP/1.1 200 OK\r\nContent-Length: 70000000\r\n\r\nx");
    assert!(out.starts_with("HTTP/1.1 500"), "{out}");
    assert!(out.contains("DRIVER_ERROR"), "{out}");
}

#[test]
fn malformed_upstream_reply_is_driver_error() {
    let out = relayed_status("garbage\r\n\r\n");
    assert!(out.starts_with("HTTP/1.1 500"), "{out}");
    assert!(out.contains("DRIVER_ERROR"), "{out}");
}

#[test]
fn duplicate_or_malformed_content_length_is_driver_error() {
    for reply in [
        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: xx\r\n\r\nhello",
        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello",
    ] {
        let out = relayed_status(reply);
        assert!(out.starts_with("HTTP/1.1 500"), "{out}");
        assert!(out.contains("DRIVER_ERROR"), "{out}");
    }
}

#[test]
fn transfer_encoding_upstream_reply_is_driver_error() {
    let out = relayed_status(
        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\nhello",
    );
    assert!(out.starts_with("HTTP/1.1 500"), "{out}");
    assert!(out.contains("DRIVER_ERROR"), "{out}");
}

#[test]
fn head_plus_body_over_cap_is_driver_error() {
    let out = relayed_status("HTTP/1.1 200 OK\r\nContent-Length: 67108864\r\n\r\n");
    assert!(out.starts_with("HTTP/1.1 500"), "{out}");
    assert!(out.contains("DRIVER_ERROR"), "{out}");
}
