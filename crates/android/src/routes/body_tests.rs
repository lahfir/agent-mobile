//! Empty-body acceptance: verbs that take no input treat a zero-length
//! body as an empty object, matching the on-device contract.

use agent_mobile_core::error::Failure;

use crate::driver::SecretToken;
use crate::http::start_bridge;
use crate::testkit::{ctl, envelope, fake_upstream, post};

const HEAD: &str = "Authorization: Bearer t0k\r\nX-Agent-Mobile-Version: 1\r\n";

#[test]
fn terminate_accepts_empty_body() -> Result<(), Failure> {
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
        "",
    );
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.contains("\"terminated\":\"com.observed\""));
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
