//! Live end-to-end adapter check against a real device: ignored by default,
//! gated on `AGENT_MOBILE_ANDROID_SERIAL` and `AGENT_MOBILE_ANDROID_TEST_PACKAGE`.

use std::time::Duration;

use agent_mobile_android::AndroidAdapter;
use agent_mobile_core::error::Failure;
use agent_mobile_core::wire::Wire;
use serde_json::json;

#[test]
#[ignore = "needs a live Android device; set AGENT_MOBILE_ANDROID_SERIAL and AGENT_MOBILE_ANDROID_TEST_PACKAGE"]
fn session_drives_real_device() -> Result<(), Failure> {
    let serial = std::env::var("AGENT_MOBILE_ANDROID_SERIAL").map_err(|_| {
        Failure::local(
            "AGENT_MOBILE_ANDROID_SERIAL unset",
            "set it to the device serial",
        )
    })?;
    let package = std::env::var("AGENT_MOBILE_ANDROID_TEST_PACKAGE").map_err(|_| {
        Failure::local(
            "AGENT_MOBILE_ANDROID_TEST_PACKAGE unset",
            "set it to a safe package",
        )
    })?;
    let adapter = AndroidAdapter::from_environment()?;
    let session = adapter.start_session(&serial)?;
    println!(
        "session up: local={} forward={}",
        session.local_port(),
        session.forward_port()
    );
    let wire = Wire::with_timeout(session.url(), session.token(), Duration::from_secs(60));
    let status = wire.call("status", &json!({}))?;
    assert!(status.ok);
    let snap = wire.call("snapshot", &json!({}))?;
    assert!(snap.ok);
    let refs = snap.data.as_ref().map_or(0, |d| match d {
        agent_mobile_core::contract::Data::Snapshot(s) => s.ref_count,
        _ => 0,
    });
    println!("snapshot refs={refs}");
    let launched = wire.call("launch", &json!({"bundle_id": package}))?;
    assert!(launched.ok);
    assert_eq!(launched.command.as_deref(), Some("launch"));
    let term = wire.call("terminate", &json!({}))?;
    assert!(term.ok);
    let after = wire.call("status", &json!({}))?;
    assert!(after.ok);
    println!("launch+terminate ok");
    session.close()?;
    println!("closed");
    Ok(())
}
