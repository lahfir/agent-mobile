//! Stub-driver coverage for the gesture verbs: `doubletap`, `pinch`, `hold`,
//! `back`, `twofinger`, and `center`, plus guide accuracy.

mod common;

use std::collections::BTreeSet;

use agent_mobile_core::error::Failure;

use common::{
    SNAPSHOT, STATUS, body_of, code, fail, run, run_wired, stderr, stdout, stub, tmp_home,
};

/// The six verbs this file covers.
const GESTURE_VERBS: [&str; 6] = ["doubletap", "pinch", "hold", "back", "twofinger", "center"];

/// The single request a wired run captured.
fn only_request(s: common::Stub, label: &str) -> Result<String, Failure> {
    let captured = s.captured()?;
    assert_eq!(captured.len(), 1, "{label}: {captured:?}");
    captured
        .into_iter()
        .next()
        .ok_or_else(|| fail("no request captured"))
}

#[test]
fn doubletap_ref_posts_path_and_ref() -> Result<(), Failure> {
    let home = tmp_home("doubletap-ref")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["doubletap", "@snap1:e2"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "doubletap-ref")?;
    assert!(req.contains("POST /doubletap "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["ref"], "@snap1:e2");
    assert!(stdout(&out).contains("snapshot=@snap1"), "{}", stdout(&out));
    Ok(())
}

#[test]
fn doubletap_xy_posts_point() -> Result<(), Failure> {
    let home = tmp_home("doubletap-xy")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["doubletap", "100", "200"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "doubletap-xy")?;
    assert!(req.contains("POST /doubletap "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["x"], 100.0);
    assert_eq!(v["y"], 200.0);
    Ok(())
}

#[test]
fn doubletap_three_args_exits_2() -> Result<(), Failure> {
    let home = tmp_home("doubletap-misuse")?;
    let out = run(&["doubletap", "a", "b", "c"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn doubletap_bad_coord_exits_2() -> Result<(), Failure> {
    let home = tmp_home("doubletap-coord")?;
    let out = run(&["doubletap", "100", "nope"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn pinch_posts_ref_and_scale_without_velocity() -> Result<(), Failure> {
    let home = tmp_home("pinch")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["pinch", "@snap1:e2", "2.0"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "pinch")?;
    assert!(req.contains("POST /pinch "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["ref"], "@snap1:e2");
    assert_eq!(v["scale"], 2.0);
    assert!(v.get("velocity").is_none(), "{v}");
    Ok(())
}

#[test]
fn pinch_with_velocity_posts_velocity() -> Result<(), Failure> {
    let home = tmp_home("pinch-vel")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(
        &["pinch", "@snap1:e2", "0.5", "--velocity", "1.5"],
        &home,
        &s,
    )?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "pinch-vel")?;
    let v = body_of(&req)?;
    assert_eq!(v["scale"], 0.5);
    assert_eq!(v["velocity"], 1.5);
    Ok(())
}

#[test]
fn pinch_missing_scale_exits_2() -> Result<(), Failure> {
    let home = tmp_home("pinch-misuse")?;
    let out = run(&["pinch", "@snap1:e2"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn pinch_bad_scale_text_exits_2() -> Result<(), Failure> {
    let home = tmp_home("pinch-text")?;
    let out = run(&["pinch", "@snap1:e2", "wide"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

/// Near-1 scales ride the wire unchanged; the driver owns the rejection.
#[test]
fn pinch_near_one_scale_passes_through() -> Result<(), Failure> {
    let home = tmp_home("pinch-near1")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["pinch", "@snap1:e2", "1.0"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "pinch-near1")?;
    assert!(req.contains("POST /pinch "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["scale"], 1.0);
    Ok(())
}

/// Non-finite scales never reach the wire: JSON cannot carry them, so the
/// CLI rejects them like `hold` rejects non-finite durations.
#[test]
fn pinch_nonfinite_scale_exits_2_without_request() -> Result<(), Failure> {
    let home = tmp_home("pinch-nan")?;
    let s = stub(&[])?;
    let out = run_wired(&["pinch", "@snap1:e2", "NaN"], &home, &s)?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert_eq!(s.captured()?.len(), 0);
    Ok(())
}

#[test]
fn pinch_nonfinite_velocity_exits_2_without_request() -> Result<(), Failure> {
    let home = tmp_home("pinch-inf-vel")?;
    let s = stub(&[])?;
    let out = run_wired(
        &["pinch", "@snap1:e2", "2.0", "--velocity", "inf"],
        &home,
        &s,
    )?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert_eq!(s.captured()?.len(), 0);
    Ok(())
}

/// A zero scale rides the wire unchanged; the driver owns the rejection.
#[test]
fn pinch_zero_scale_passes_through() -> Result<(), Failure> {
    let home = tmp_home("pinch-zero")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["pinch", "@snap1:e2", "0"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "pinch-zero")?;
    assert!(req.contains("POST /pinch "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["scale"], 0.0);
    Ok(())
}

#[test]
fn hold_ref_defaults_duration_to_one() -> Result<(), Failure> {
    let home = tmp_home("hold-ref")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["hold", "@snap1:e2"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "hold-ref")?;
    assert!(req.contains("POST /hold "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["ref"], "@snap1:e2");
    assert_eq!(v["duration"], 1.0);
    Ok(())
}

#[test]
fn hold_xy_with_duration_posts_point_and_duration() -> Result<(), Failure> {
    let home = tmp_home("hold-xy")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["hold", "100", "200", "--duration", "1.5"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "hold-xy")?;
    let v = body_of(&req)?;
    assert_eq!(v["x"], 100.0);
    assert_eq!(v["y"], 200.0);
    assert_eq!(v["duration"], 1.5);
    Ok(())
}

#[test]
fn hold_without_target_exits_2() -> Result<(), Failure> {
    let home = tmp_home("hold-misuse")?;
    let out = run(&["hold"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn hold_bad_duration_text_exits_2() -> Result<(), Failure> {
    let home = tmp_home("hold-duration")?;
    let out = run(&["hold", "@snap1:e2", "--duration", "long"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn hold_nonfinite_duration_exits_2_without_request() -> Result<(), Failure> {
    let home = tmp_home("hold-inf")?;
    let s = stub(&[])?;
    let out = run_wired(&["hold", "100", "200", "--duration", "inf"], &home, &s)?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert_eq!(s.captured()?.len(), 0);
    Ok(())
}

/// Zero and negative durations ride the wire unchanged; positivity is the
/// driver's call, so a later client-side check would break this test first.
#[test]
fn hold_zero_duration_passes_through() -> Result<(), Failure> {
    let home = tmp_home("hold-zero")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["hold", "@snap1:e2", "--duration", "0"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "hold-zero")?;
    let v = body_of(&req)?;
    assert_eq!(v["duration"], 0.0);
    Ok(())
}

#[test]
fn hold_negative_duration_passes_through() -> Result<(), Failure> {
    let home = tmp_home("hold-neg")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["hold", "@snap1:e2", "--duration", "-1"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "hold-neg")?;
    let v = body_of(&req)?;
    assert_eq!(v["duration"], -1.0);
    Ok(())
}

#[test]
fn back_posts_empty_body_and_driver_stays_up() -> Result<(), Failure> {
    let home = tmp_home("back")?;
    let s = stub(&[SNAPSHOT, STATUS])?;
    let out = run_wired(&["back"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = run_wired(&["status"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    assert_eq!(captured.len(), 2, "{captured:?}");
    let req = captured.first().ok_or_else(|| fail("no request"))?;
    assert!(req.contains("POST /back "), "{req}");
    let v = body_of(req)?;
    assert_eq!(v, serde_json::json!({}));
    Ok(())
}

#[test]
fn back_with_arg_exits_2() -> Result<(), Failure> {
    let home = tmp_home("back-misuse")?;
    let out = run(&["back", "extra"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn twofinger_posts_ref() -> Result<(), Failure> {
    let home = tmp_home("twofinger")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["twofinger", "@snap1:e2"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "twofinger")?;
    assert!(req.contains("POST /twofinger "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["ref"], "@snap1:e2");
    Ok(())
}

#[test]
fn twofinger_without_ref_exits_2() -> Result<(), Failure> {
    let home = tmp_home("twofinger-misuse")?;
    let out = run(&["twofinger"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn twofinger_bad_ref_exits_2() -> Result<(), Failure> {
    let home = tmp_home("twofinger-badref")?;
    let out = run(&["twofinger", "nope"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

#[test]
fn center_notification_posts_which() -> Result<(), Failure> {
    let home = tmp_home("center")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["center", "notification"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let req = only_request(s, "center")?;
    assert!(req.contains("POST /center "), "{req}");
    let v = body_of(&req)?;
    assert_eq!(v["which"], "notification");
    Ok(())
}

#[test]
fn center_other_value_exits_2() -> Result<(), Failure> {
    let home = tmp_home("center-misuse")?;
    let out = run(&["center", "control"], &home, &[])?;
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    Ok(())
}

/// Command names under a header block. The block starts at the line
/// beginning with `header` and ends at the first line where `ends_block`
/// holds; rows satisfying `is_row` contribute their first token.
fn command_names(
    text: &str,
    header: &str,
    ends_block: impl Fn(&str) -> bool,
    is_row: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut in_commands = false;
    for line in text.lines() {
        if line.starts_with(header) {
            in_commands = true;
            continue;
        }
        if in_commands {
            if ends_block(line) {
                break;
            }
            if is_row(line)
                && let Some(name) = line.split_whitespace().next()
            {
                names.insert(name.to_owned());
            }
        }
    }
    names
}

/// Command rows indent exactly two spaces; deeper lines are descriptions.
fn names_in_help() -> Result<BTreeSet<String>, Failure> {
    let home = tmp_home("gestures-help")?;
    let out = run(&["--help"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    Ok(command_names(
        &text,
        "Commands:",
        |line| line.trim().is_empty() || line.starts_with("Options:"),
        |line| {
            line.split_whitespace()
                .next()
                .is_some_and(|name| name != "help")
        },
    ))
}

/// Guide rows indent exactly two spaces; deeper lines are continuations.
fn names_in_skills() -> Result<BTreeSet<String>, Failure> {
    let home = tmp_home("gestures-skills")?;
    let out = run(&["skills"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    Ok(command_names(
        &text,
        "COMMANDS",
        |line| line.trim().is_empty() || !line.starts_with("  "),
        |line| {
            line.starts_with("  ")
                && !line.starts_with("   ")
                && line.split_whitespace().next().is_some()
        },
    ))
}

#[test]
fn help_lists_the_new_verbs() -> Result<(), Failure> {
    let help = names_in_help()?;
    for verb in GESTURE_VERBS {
        assert!(help.contains(verb), "help omits {verb}");
    }
    Ok(())
}

#[test]
fn guide_names_every_new_verb_and_back() -> Result<(), Failure> {
    let help = names_in_help()?;
    let skills = names_in_skills()?;
    for verb in GESTURE_VERBS {
        assert!(skills.contains(verb), "guide omits {verb}");
        assert!(help.contains(verb), "guide names {verb} with no command");
    }
    Ok(())
}
