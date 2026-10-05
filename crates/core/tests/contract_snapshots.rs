//! Golden contract tests: driver-shaped JSON parses into the typed contract,
//! and every registry code renders its verbatim code plus its next action.

use agent_mobile_core::contract::{Bounds, Data, Envelope, ErrorBody, Snapshot};
use agent_mobile_core::error::{EXIT_ERROR, EXIT_USAGE, ErrorCode, Failure};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SETTLED_SNAPSHOT: &str = r#"{
  "version": "1",
  "ok": true,
  "command": "snapshot",
  "elapsed_ms": 5101,
  "data": {
    "app": "com.example.todo",
    "snapshot_id": "upii2see",
    "ref_count": 3,
    "complete": true,
    "settled": true,
    "reads": 2,
    "text": "@upii2see:e1 application \"ToDo\" at=0,0 size=440x956\n  @upii2see:e2 textfield \"Title\" value=\"\" at=16,120 size=408x44 [focused]",
    "tree": {
      "role": "application",
      "name": "ToDo",
      "value": "",
      "ref_id": "@upii2see:e1",
      "states": [],
      "available_actions": [],
      "bounds": {"x": 0.0, "y": 0.0, "width": 440.0, "height": 956.0},
      "children": [
        {
          "role": "textfield",
          "name": "Title",
          "value": "",
          "ref_id": "@upii2see:e2",
          "states": ["focused"],
          "available_actions": ["Tap", "Type", "Clear"],
          "bounds": {"x": 16.0, "y": 120.0, "width": 408.0, "height": 44.0},
          "native_id": {"kind": "ax_identifier", "value": "todo.title"},
          "children": []
        },
        {
          "role": "button",
          "name": "Add",
          "value": "",
          "ref_id": "@upii2see:e3",
          "states": ["disabled"],
          "available_actions": ["Tap"],
          "bounds": {"x": 388.0, "y": 52.0, "width": 44.0, "height": 44.0},
          "children": []
        }
      ]
    }
  }
}"#;

const STATUS: &str = r#"{
  "version": "1",
  "ok": true,
  "command": "status",
  "elapsed_ms": 3,
  "data": {
    "app": "com.example.todo",
    "snapshot_id": "upii2see",
    "device": "iPhone 16 Pro",
    "os": "26.0"
  }
}"#;

const TERMINATE: &str = r#"{
  "version": "1",
  "ok": true,
  "command": "terminate",
  "elapsed_ms": 812,
  "data": {"terminated": "com.example.todo"}
}"#;

const SCREENSHOT: &str = r#"{
  "version": "1",
  "ok": true,
  "command": "screenshot",
  "elapsed_ms": 120,
  "data": {"png_base64": "iVBORw0KGgoAAAANSUhEUg"}
}"#;

const UNAUTHORIZED: &str = r#"{
  "version": "1",
  "ok": false,
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Authorization: Bearer <AGENT_MOBILE_TOKEN> required"
  }
}"#;

const OLD_PROTOCOL: &str = r#"{
  "version": "0.1-probe",
  "ok": true,
  "command": "status",
  "elapsed_ms": 3,
  "data": {
    "app": "com.example.todo",
    "snapshot_id": "upii2see",
    "device": "iPhone 16 Pro",
    "os": "26.0"
  }
}"#;

#[test]
fn settled_snapshot_envelope_parses() -> TestResult {
    let env = Envelope::from_json(SETTLED_SNAPSHOT)?;
    assert_eq!(env.version, "1");
    assert!(env.ok);
    assert_eq!(env.command.as_deref(), Some("snapshot"));
    assert_eq!(env.elapsed_ms, Some(5101));
    assert!(env.error.is_none());
    assert!(env.check_version().is_ok());
    let Some(Data::Snapshot(snap)) = env.data else {
        return Err("expected the snapshot data variant".into());
    };
    check_settled_snapshot(&snap);
    insta::assert_debug_snapshot!("settled_snapshot_data", snap);
    Ok(())
}

/// The settled snapshot's fixed fields, kept out of the envelope test so
/// neither trips the complexity lint.
fn check_settled_snapshot(snap: &Snapshot) {
    assert_eq!(snap.app, "com.example.todo");
    assert_eq!(snap.snapshot_id, "upii2see");
    assert_eq!(snap.ref_count, 3);
    assert!(snap.complete && snap.settled);
    assert_eq!(snap.reads, 2);
    assert_eq!(
        snap.tree.bounds,
        Bounds {
            x: 0.0,
            y: 0.0,
            width: 440.0,
            height: 956.0,
        }
    );
    assert_eq!(snap.tree.children.len(), 2);
}

#[test]
fn status_reply_parses() -> TestResult {
    let env = Envelope::from_json(STATUS)?;
    let Some(Data::Status(status)) = env.data else {
        return Err("expected the status data variant".into());
    };
    assert_eq!(status.device, "iPhone 16 Pro");
    insta::assert_debug_snapshot!("status_data", status);
    Ok(())
}

#[test]
fn terminate_reply_parses() -> TestResult {
    let env = Envelope::from_json(TERMINATE)?;
    let Some(Data::Terminate(terminate)) = env.data else {
        return Err("expected the terminate data variant".into());
    };
    insta::assert_debug_snapshot!("terminate_data", terminate);
    Ok(())
}

#[test]
fn screenshot_reply_parses() -> TestResult {
    let env = Envelope::from_json(SCREENSHOT)?;
    let Some(Data::Screenshot(screenshot)) = env.data else {
        return Err("expected the screenshot data variant".into());
    };
    insta::assert_debug_snapshot!("screenshot_data", screenshot);
    Ok(())
}

#[test]
fn unauthorized_401_maps_to_token_hint() -> TestResult {
    let env = Envelope::from_json(UNAUTHORIZED)?;
    assert!(!env.ok);
    assert!(env.command.is_none());
    assert!(env.elapsed_ms.is_none());
    assert!(env.data.is_none());
    let Some(body) = env.error else {
        return Err("expected an error body".into());
    };
    assert_eq!(
        body,
        ErrorBody {
            code: String::from("UNAUTHORIZED"),
            message: String::from("Authorization: Bearer <AGENT_MOBILE_TOKEN> required"),
        }
    );
    let failure = Failure::from_error_body(&body);
    assert_eq!(failure.exit_code(), EXIT_ERROR);
    let rendered = failure.render();
    assert!(rendered.contains("UNAUTHORIZED"));
    assert!(rendered.contains("AGENT_MOBILE_TOKEN"));
    insta::assert_snapshot!("unauthorized_render", rendered);
    Ok(())
}

#[test]
fn all_codes_render_verbatim_hints() {
    for code in ErrorCode::ALL {
        let rendered = Failure::driver(code, "driver message").render();
        assert!(rendered.contains(code.as_str()));
        assert!(rendered.contains(code.next_action()));
        insta::assert_snapshot!(code.as_str(), rendered);
    }
}

#[test]
fn driver_error_envelope_retries_once() {
    let rendered = Failure::driver(ErrorCode::DriverError, "the runner exploded").render();
    assert!(rendered.contains("DRIVER_ERROR"));
    assert!(rendered.contains("retry once"));
    assert!(!rendered.contains("checklist"));
    insta::assert_snapshot!("driver_error_render", rendered);
}

#[test]
fn unknown_code_maps_to_driver_error() {
    let failure = Failure::from_error_body(&ErrorBody {
        code: String::from("FUTURE_CODE"),
        message: String::from("new driver behavior"),
    });
    assert_eq!(failure.exit_code(), EXIT_ERROR);
    let rendered = failure.render();
    assert!(rendered.contains("DRIVER_ERROR"));
    assert!(rendered.contains("new driver behavior"));
}

#[test]
fn transport_failure_escalates_with_checklist() {
    let failure = Failure::transport("connection refused (os error 61)");
    assert_eq!(failure.exit_code(), EXIT_ERROR);
    let rendered = failure.render();
    assert!(rendered.contains("DRIVER_ERROR"));
    assert!(rendered.contains("agent-mobile serve"));
    assert!(rendered.contains("Wi-Fi"));
    assert!(rendered.contains("no retry loop"));
    insta::assert_snapshot!("transport_escalation", rendered);
}

#[test]
fn version_mismatch_is_fatal_upgrade() -> TestResult {
    let env = Envelope::from_json(OLD_PROTOCOL)?;
    match env.check_version() {
        Ok(()) => Err("expected a fatal version mismatch".into()),
        Err(failure) => {
            assert_eq!(failure.exit_code(), EXIT_ERROR);
            insta::assert_snapshot!("version_mismatch", failure.render());
            Ok(())
        }
    }
}

#[test]
fn usage_failures_exit_2() {
    let failure = Failure::usage("tap expects <ref> or <x> <y>");
    assert_eq!(failure.exit_code(), EXIT_USAGE);
    insta::assert_snapshot!("usage_render", failure.render());
}

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
