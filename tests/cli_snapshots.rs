//! End-to-end CLI tests: the built binary against stub HTTP servers, one
//! isolated HOME per test, covering the argument shapes, output contract,
//! and exit codes.

mod common;

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

use common::{
    SNAPSHOT, STALE, STATUS, body_of, code, fail, run, run_wired, stderr, stdout, stub, tmp_home,
};

#[test]
fn tap_one_arg_sends_ref() -> Result<(), Failure> {
    let home = tmp_home("tap-ref")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["tap", "@abc:e1"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["ref"], "@abc:e1");
    assert!(v.get("x").is_none());
    Ok(())
}

#[test]
fn tap_two_args_send_float_coordinates() -> Result<(), Failure> {
    let home = tmp_home("tap-xy")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["tap", "10.5", "20"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["x"], 10.5);
    assert_eq!(v["y"], 20.0);
    assert!(v.get("ref").is_none());
    Ok(())
}

#[test]
fn tap_with_no_args_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("tap-none")?;
    let out = run(&["tap"], &home, &[])?;
    assert_eq!(code(&out), 2);
    Ok(())
}

#[test]
fn tap_bad_coordinate_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("tap-bad")?;
    let out = run(&["tap", "10", "abc"], &home, &[])?;
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("x y"), "{}", stderr(&out));
    Ok(())
}

#[test]
fn type_consumes_leading_ref_then_joins_text() -> Result<(), Failure> {
    let home = tmp_home("type-ref")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["type", "@abc:e1", "hello", "world"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["ref"], "@abc:e1");
    assert_eq!(v["text"], "hello world");
    Ok(())
}

#[test]
fn type_dash_text_passes_through_after_double_dash() -> Result<(), Failure> {
    let home = tmp_home("type-dash")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["type", "--", "-flag"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["text"], "-flag");
    assert!(v.get("ref").is_none());
    Ok(())
}

#[test]
fn type_hyphen_arg_without_escape_is_rejected() -> Result<(), Failure> {
    let home = tmp_home("type-hyphen")?;
    let out = run(&["type", "-flag"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn type_does_not_eat_trailing_global_flags() -> Result<(), Failure> {
    let home = tmp_home("type-flags")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["type", "hello", "--json"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let line = stdout(&out);
    let v: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| fail(&format!("{e}: {line}")))?;
    assert_eq!(v["ok"], true, "{v}");
    let captured = s.captured()?;
    let body = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(body["text"], "hello");
    Ok(())
}

#[test]
fn type_at_mention_is_text_not_ref() -> Result<(), Failure> {
    let home = tmp_home("type-at")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["type", "@handle", "hi"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert!(v.get("ref").is_none(), "{v}");
    assert_eq!(v["text"], "@handle hi");
    Ok(())
}

#[test]
fn type_ref_without_text_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("type-bare")?;
    let out = run(&["type", "@abc:e1"], &home, &[])?;
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("text"), "{}", stderr(&out));
    Ok(())
}

#[test]
fn swipe_rejects_bad_direction_before_network() -> Result<(), Failure> {
    let home = tmp_home("swipe-bad")?;
    let out = run(&["swipe", "diagonal"], &home, &[])?;
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("up"), "{err}");
    assert!(err.contains("down"), "{err}");
    Ok(())
}

#[test]
fn swipe_direction_and_ref_reach_body() -> Result<(), Failure> {
    let home = tmp_home("swipe-ok")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["swipe", "left", "@abc:e2"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["direction"], "left");
    assert_eq!(v["ref"], "@abc:e2");
    Ok(())
}

#[test]
fn json_flag_passes_the_envelope() -> Result<(), Failure> {
    let home = tmp_home("json-ok")?;
    let s = stub(&[STATUS])?;
    let out = run_wired(&["status", "--json"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let got: serde_json::Value = serde_json::from_str(stdout(&out).trim())
        .map_err(|e| fail(&format!("json stdout: {e}")))?;
    let want: serde_json::Value =
        serde_json::from_str(STATUS).map_err(|e| fail(&format!("fixture: {e}")))?;
    assert_eq!(got, want);
    Ok(())
}

#[test]
fn json_flag_passes_error_envelopes_with_exit_1() -> Result<(), Failure> {
    let home = tmp_home("json-err")?;
    let s = stub(&[STALE])?;
    let out = run_wired(&["tap", "@abc:e1", "--json"], &home, &s)?;
    assert_eq!(code(&out), 1);
    let got: serde_json::Value = serde_json::from_str(stdout(&out).trim())
        .map_err(|e| fail(&format!("json stdout: {e}")))?;
    assert_eq!(got["error"]["code"], "STALE_REF");
    Ok(())
}

#[test]
fn text_error_goes_to_stderr_leaving_stdout_parseable() -> Result<(), Failure> {
    let home = tmp_home("stderr")?;
    let s = stub(&[STALE])?;
    let out = run_wired(&["tap", "@abc:e1"], &home, &s)?;
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).is_empty(), "{}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("STALE_REF"), "{err}");
    assert!(err.contains("next:"), "{err}");
    Ok(())
}

#[test]
fn app_flag_reaches_snapshot_body() -> Result<(), Failure> {
    let home = tmp_home("app")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["snapshot", "--app", "com.foo"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["app"], "com.foo");
    Ok(())
}

#[test]
fn max_depth_trims_tree_and_marks_incomplete() -> Result<(), Failure> {
    let home = tmp_home("depth")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["snapshot", "--max-depth", "1"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("complete=false"), "{text}");
    assert!(text.contains("Outer"), "{text}");
    assert!(!text.contains("Deep"), "{text}");
    Ok(())
}

#[test]
fn device_flag_is_remembered_and_routes_next_call() -> Result<(), Failure> {
    let home = tmp_home("device")?;
    let s = stub(&[STATUS, STATUS])?;
    let store = StateStore::at(&home.join(".agent-mobile"));
    store.write_token("sim", "tok")?;
    store.upsert(
        "sim",
        &SessionEntry::new(s.url.clone(), std::process::id(), "sim".to_owned()),
    )?;
    let out = run(&["status", "--device", "sim"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = run(&["status"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    assert_eq!(captured.len(), 2, "{captured:?}");
    let state = store.load();
    assert_eq!(state.default_device.as_deref(), Some("sim"));
    Ok(())
}

#[test]
fn screenshot_path_and_json_conflict_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("shot-conflict")?;
    let out = run(&["screenshot", "/tmp/x.png", "--json"], &home, &[])?;
    assert_eq!(code(&out), 2);
    let line = stdout(&out);
    let v: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| fail(&format!("{e}: {line}")))?;
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["error"]["code"], "USAGE");
    Ok(())
}

#[test]
fn json_mode_failures_emit_envelopes_on_stdout() -> Result<(), Failure> {
    let home = tmp_home("json-err")?;
    let out = run(
        &["status", "--json"],
        &home,
        &[
            ("AGENT_MOBILE_URL", "http://127.0.0.1:1"),
            ("AGENT_MOBILE_TOKEN", "tok"),
        ],
    )?;
    assert_eq!(code(&out), 1);
    let line = stdout(&out);
    let v: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| fail(&format!("{e}: {line}")))?;
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["version"], "1");
    assert_eq!(v["command"], "status");
    assert_eq!(v["error"]["code"], "DRIVER_ERROR", "{v}");
    assert!(
        stderr(&out).contains("next:"),
        "stderr keeps the human render"
    );
    Ok(())
}

#[test]
fn unreachable_driver_names_the_remedy() -> Result<(), Failure> {
    let home = tmp_home("nosession")?;
    let out = run(
        &["status"],
        &home,
        &[
            ("AGENT_MOBILE_URL", "http://127.0.0.1:1"),
            ("AGENT_MOBILE_TOKEN", "tok"),
        ],
    )?;
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(err.contains("serve"), "{err}");
    Ok(())
}

#[test]
fn stale_ref_from_driver_exits_1_with_resnapshot_hint() -> Result<(), Failure> {
    let home = tmp_home("stale")?;
    let s = stub(&[STALE])?;
    let out = run_wired(&["tap", "@older:e1"], &home, &s)?;
    assert_eq!(code(&out), 1);
    let captured = s.captured()?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["ref"], "@older:e1");
    let err = stderr(&out);
    assert!(err.contains("STALE_REF"), "{err}");
    assert!(err.contains("re-snapshot"), "{err}");
    Ok(())
}

#[test]
fn status_text_output_snapshot() -> Result<(), Failure> {
    let home = tmp_home("snap-status")?;
    let s = stub(&[STATUS])?;
    let out = run_wired(&["status"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    insta::assert_snapshot!(
        stdout(&out).trim_end(),
        @"app=com.apple.springboard device=\"Sim\" os=26.0 snapshot=@sid1 elapsed_ms=3"
    );
    Ok(())
}

#[test]
fn snapshot_text_output_snapshot() -> Result<(), Failure> {
    let home = tmp_home("snap-tree")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["snapshot"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    insta::assert_snapshot!(stdout(&out), @r###"
    app=com.x snapshot=@snap1 refs=2 settled=true reads=2 elapsed_ms=7
    @snap1:e1 application "App" at=0,0 size=430x930
      @snap1:e2 group "Outer" at=0,0 size=100x50
        @snap1:e3 button "Deep" at=1,2 size=10x10
    "###);
    Ok(())
}
