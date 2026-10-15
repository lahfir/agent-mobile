//! Recorded Android fixtures: bytes a real driver answered, never
//! hand-written — every one parses as a v1 envelope with its exact
//! shape, codes, and `resource_id` native ids.

use agent_mobile_core::contract::{Data, Envelope};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Recorded-fixture loader: bytes a real driver answered, never hand-written.
fn recorded_fixture(name: &str) -> Result<Envelope, Box<dyn std::error::Error>> {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path)?;
    Ok(Envelope::from_json(&raw)?)
}

#[test]
fn android_fixtures_parse_as_v1_envelopes() -> TestResult {
    for name in [
        "android-status",
        "android-snapshot",
        "android-error-bad-request",
        "android-error-unauthorized",
        "android-error-version-mismatch",
        "android-error-unknown-command",
        "android-error-stale",
        "android-error-ambiguous",
        "android-screenshot",
        "android-terminate",
    ] {
        let env = recorded_fixture(name)?;
        assert_eq!(
            env.version,
            agent_mobile_core::contract::PROTOCOL_VERSION,
            "{name} must speak protocol v1"
        );
    }
    Ok(())
}

/// Depth-first search for one Android `resource_id` native id.
fn has_resource_id(node: &agent_mobile_core::contract::Node) -> bool {
    node.native_id
        .as_ref()
        .is_some_and(|n| n.kind == "resource_id" && !n.value.is_empty())
        || node.children.iter().any(has_resource_id)
}

#[test]
fn android_snapshot_is_real_and_resource_id_backed() -> TestResult {
    let env = recorded_fixture("android-snapshot")?;
    assert_eq!(env.command.as_deref(), Some("snapshot"));
    let Some(Data::Snapshot(snap)) = env.data else {
        return Err("expected Data::Snapshot".into());
    };
    assert!(snap.ref_count > 0);
    assert!(!snap.text.is_empty());
    assert_eq!(
        agent_mobile_core::format::tree_lines(&snap.tree),
        snap.text,
        "driver text must equal the tree render"
    );
    assert!(
        has_resource_id(&snap.tree),
        "Android trees must carry resource_id native ids"
    );
    Ok(())
}

#[test]
fn android_status_and_terminate_shapes() -> TestResult {
    let env = recorded_fixture("android-status")?;
    assert_eq!(env.command.as_deref(), Some("status"));
    let Some(Data::Status(s)) = env.data else {
        return Err("expected Data::Status".into());
    };
    assert!(!s.app.is_empty() && !s.device.is_empty() && !s.os.is_empty());
    let env = recorded_fixture("android-terminate")?;
    assert_eq!(env.command.as_deref(), Some("terminate"));
    let Some(Data::Terminate(t)) = env.data else {
        return Err("expected Data::Terminate".into());
    };
    assert!(!t.terminated.is_empty());
    Ok(())
}

#[test]
fn android_error_fixtures_carry_exact_codes() -> TestResult {
    let cases = [
        ("android-error-bad-request", "BAD_REQUEST"),
        ("android-error-version-mismatch", "BAD_REQUEST"),
        ("android-error-unknown-command", "UNKNOWN_COMMAND"),
        ("android-error-stale", "STALE_REF"),
        ("android-error-ambiguous", "AMBIGUOUS_TARGET"),
    ];
    for (name, code) in cases {
        let env = recorded_fixture(name)?;
        assert!(!env.ok && env.data.is_none(), "{name} must be a bare error");
        let body = env
            .error
            .ok_or_else(|| format!("{name} must be an error envelope"))?;
        assert_eq!(body.code, code, "{name}");
    }
    let env = recorded_fixture("android-error-unauthorized")?;
    assert!(!env.ok && env.data.is_none());
    assert!(env.command.is_none() && env.elapsed_ms.is_none());
    let body = env
        .error
        .ok_or("unauthorized fixture must carry an error")?;
    assert_eq!(body.code, "UNAUTHORIZED");
    Ok(())
}

#[test]
fn android_screenshot_decodes_to_png() -> TestResult {
    use base64::Engine as _;
    let env = recorded_fixture("android-screenshot")?;
    let Some(Data::Screenshot(s)) = env.data else {
        return Err("expected Data::Screenshot".into());
    };
    assert!(s.png_base64.len() <= 2048 && s.png_base64.len() % 4 == 0);
    let png = base64::engine::general_purpose::STANDARD.decode(&s.png_base64)?;
    assert_eq!(
        &png[..8],
        b"\x89PNG\r\n\x1a\n",
        "fixture must start with PNG magic"
    );
    Ok(())
}
