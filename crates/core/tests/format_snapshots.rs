//! Golden tests over recorded driver fixtures (KTD4, KTD15): rendered output
//! must equal the driver's own text mode byte-for-byte, and redactions keep
//! dynamic values (snapshot ids, timings, ports, paths) out of snapshots.

use agent_mobile_core::contract::{Envelope, Ref, trim_snapshot};
use agent_mobile_core::error::{EXIT_USAGE, Failure};
use agent_mobile_core::format::{render, screenshot_written};

mod common;
use common::fail;

fn fixture(name: &str) -> Result<Envelope, Failure> {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path)?;
    Envelope::from_json(&raw)
        .map_err(|e| Failure::local(format!("fixture {name}: {e}"), "re-record fixtures"))
}

fn render_fixture(name: &str) -> Result<String, Failure> {
    let env = fixture(name)?;
    if let Some(err) = &env.error {
        return Ok(Failure::from_error_body(err).render());
    }
    Ok(render(&env).into_owned())
}

macro_rules! snap {
    ($name:expr, $text:expr) => {
        insta::with_settings!({filters => vec![
            (r"snapshot=@[a-z0-9]+", "snapshot=@SNAP"),
            (r"@[a-z0-9]+:e\d+", "@SNAP:eN"),
            (r"elapsed_ms=\d+", "elapsed_ms=N"),
            (r"127\.0\.0\.1:\d+", "127.0.0.1:PORT"),
        ]}, {
            insta::assert_snapshot!($name, $text)
        })
    };
}

fn expect_snapshot(env: &Envelope) -> Result<&agent_mobile_core::contract::Snapshot, Failure> {
    match &env.data {
        Some(agent_mobile_core::contract::Data::Snapshot(s)) => Ok(s),
        _ => Err(fail("fixture is not a snapshot")),
    }
}

fn driver_header(env: &Envelope, snap: &agent_mobile_core::contract::Snapshot) -> String {
    format!(
        "app={} snapshot=@{} refs={} settled={} reads={} elapsed_ms={}",
        snap.app,
        snap.snapshot_id,
        snap.ref_count,
        snap.settled,
        snap.reads,
        env.elapsed_ms.unwrap_or_default()
    )
}

#[test]
fn calendar_render_matches_driver_text_byte_for_byte() -> Result<(), Failure> {
    let env = fixture("snapshot-calendar")?;
    let snap = expect_snapshot(&env)?;
    let expected = format!("{}\n{}\n", driver_header(&env, snap), snap.text);
    assert_eq!(
        render(&env),
        expected,
        "formatter must reproduce the driver's text mode"
    );
    Ok(())
}

#[test]
fn springboard_render_matches_driver_text_byte_for_byte() -> Result<(), Failure> {
    let env = fixture("snapshot-springboard")?;
    let snap = expect_snapshot(&env)?;
    let expected = format!("{}\n{}\n", driver_header(&env, snap), snap.text);
    assert_eq!(
        render(&env),
        expected,
        "formatter must reproduce the driver's text mode"
    );
    Ok(())
}

#[test]
fn calendar_header_line_matches_live_format() -> Result<(), Failure> {
    let env = fixture("snapshot-calendar")?;
    let first = render(&env)
        .lines()
        .next()
        .map(str::to_owned)
        .unwrap_or_default();
    insta::with_settings!({filters => vec![
        (r"snapshot=@[a-z0-9]+", "snapshot=@SNAP"),
        (r"elapsed_ms=\d+", "elapsed_ms=N"),
    ]}, {
        insta::assert_snapshot!(first);
    });
    let snap = expect_snapshot(&env)?;
    assert!(first.starts_with(&format!("app={} snapshot=@", snap.app)));
    Ok(())
}

#[test]
fn stale_ref_fixture_renders_resnapshot_hint() -> Result<(), Failure> {
    let text = render_fixture("error-stale")?;
    assert!(text.contains("STALE_REF"));
    assert!(text.contains("re-snapshot"));
    snap!("stale_ref", text);
    Ok(())
}

#[test]
fn ambiguous_fixture_renders_executable_recipe() -> Result<(), Failure> {
    let text = render_fixture("error-ambiguous")?;
    assert!(text.contains("AMBIGUOUS_TARGET"));
    assert!(text.contains("firmer native_id"));
    snap!("ambiguous", text);
    Ok(())
}

#[test]
fn bad_request_fixture_renders() -> Result<(), Failure> {
    let text = render_fixture("error-bad-request")?;
    assert!(text.contains("BAD_REQUEST"));
    snap!("bad_request", text);
    Ok(())
}

#[test]
fn unauthorized_fixture_renders_token_fix() -> Result<(), Failure> {
    let text = render_fixture("error-unauthorized")?;
    assert!(text.contains("UNAUTHORIZED"));
    assert!(text.contains("AGENT_MOBILE_TOKEN"));
    snap!("unauthorized", text);
    Ok(())
}

#[test]
fn unknown_command_fixture_renders() -> Result<(), Failure> {
    let text = render_fixture("error-unknown-command")?;
    assert!(text.contains("UNKNOWN_COMMAND"));
    snap!("unknown_command", text);
    Ok(())
}

#[test]
fn version_mismatch_fixture_renders() -> Result<(), Failure> {
    let text = render_fixture("error-version-mismatch")?;
    assert!(text.contains("BAD_REQUEST"));
    snap!("version_mismatch_refusal", text);
    Ok(())
}

#[test]
fn status_renders_one_line() -> Result<(), Failure> {
    let text = render_fixture("status")?;
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("app=com.apple.springboard"));
    snap!("status_line", text);
    Ok(())
}

#[test]
fn terminate_renders_one_line() -> Result<(), Failure> {
    let text = render_fixture("terminate")?;
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("terminated=com.apple.mobilecal"));
    snap!("terminate_line", text);
    Ok(())
}

#[test]
fn screenshot_decodes_and_file_line_counts_bytes() -> Result<(), Failure> {
    let env = fixture("screenshot")?;
    let agent_mobile_core::contract::Data::Screenshot(shot) =
        env.data.clone().ok_or_else(|| fail("missing data"))?
    else {
        return Err(fail("expected Screenshot data"));
    };
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &shot.png_base64)
            .map_err(|e| fail(&format!("fixture screenshot: {e}")))?;
    assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47], "PNG magic expected");
    assert_eq!(render(&env), shot.png_base64);
    let line = screenshot_written("/tmp/shot.png", bytes.len());
    assert!(line.contains("bytes="));
    snap!("screenshot_line", line);
    Ok(())
}

#[test]
fn max_depth_trims_and_marks_incomplete() -> Result<(), Failure> {
    let mut env = fixture("snapshot-calendar")?;
    let before = render(&env).lines().count();
    let Some(agent_mobile_core::contract::Data::Snapshot(snap)) = &mut env.data else {
        return Err(fail("expected Snapshot data"));
    };
    trim_snapshot(snap, 1);
    assert!(!snap.complete);
    let after = render(&env).into_owned();
    assert!(
        after.lines().count() < before,
        "trimming must drop deep lines"
    );
    assert!(after.contains("complete=false"));
    let snap = expect_snapshot(&env)?;
    assert_eq!(snap.ref_count, 127, "header counts stay consistent");
    snap!("calendar_max_depth_1", after);
    Ok(())
}

#[test]
fn ref_parses_and_display_round_trips() -> Result<(), Failure> {
    let r = Ref::parse("@29dtut2j:e15")?;
    assert_eq!(r.snapshot_id, "29dtut2j");
    assert_eq!(r.index, 15);
    assert_eq!(r.to_string(), "@29dtut2j:e15");
    Ok(())
}

#[test]
fn malformed_refs_are_usage_errors() -> Result<(), Failure> {
    for bad in ["e15", "@x:y", "@:e1", "@x:e", "@x", "x:e1", ""] {
        match Ref::parse(bad) {
            Err(f) => assert_eq!(f.exit_code(), EXIT_USAGE, "{bad:?} must exit 2"),
            Ok(_) => return Err(fail(&format!("{bad:?} must not parse as a ref"))),
        }
    }
    Ok(())
}
