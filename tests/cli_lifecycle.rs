//! Lifecycle verbs and the skills guide: launch, activate, home, stop,
//! screenshot routing, and the two-way help/guide accuracy check.

mod common;

use std::collections::BTreeSet;
use std::io::Read;

use agent_mobile_core::error::Failure;

use common::{
    SNAPSHOT, STATUS, body_of, code, fail, run, run_wired, stderr, stdout, stub, tmp_home,
};

const TERMINATE: &str = r#"{"version":"1","ok":true,"command":"terminate","elapsed_ms":5,"data":{"terminated":"com.x"}}"#;

const SCREENSHOT: &str = r#"{"version":"1","ok":true,"command":"screenshot","elapsed_ms":9,"data":{"png_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="}}"#;

#[test]
fn launch_sends_bundle_id_and_returns_snapshot() -> Result<(), Failure> {
    let home = tmp_home("launch")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["launch", "com.apple.mobilecal"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let req = captured.first().ok_or_else(|| fail("no request"))?;
    assert!(req.contains("POST /launch "), "{req}");
    let v = body_of(req)?;
    assert_eq!(v["bundle_id"], "com.apple.mobilecal");
    assert!(stdout(&out).contains("snapshot=@snap1"), "{}", stdout(&out));
    Ok(())
}

#[test]
fn activate_sends_bundle_id_and_returns_snapshot() -> Result<(), Failure> {
    let home = tmp_home("activate")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["activate", "com.x"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let req = captured.first().ok_or_else(|| fail("no request"))?;
    assert!(req.contains("POST /activate "), "{req}");
    let v = body_of(req)?;
    assert_eq!(v["bundle_id"], "com.x");
    Ok(())
}

#[test]
fn home_returns_springboard_tree() -> Result<(), Failure> {
    let home = tmp_home("home")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["home"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    let req = captured.first().ok_or_else(|| fail("no request"))?;
    assert!(req.contains("POST /home "), "{req}");
    assert!(stdout(&out).contains("app=com.x"), "{}", stdout(&out));
    Ok(())
}

#[test]
fn stop_prints_terminate_and_driver_stays_up() -> Result<(), Failure> {
    let home = tmp_home("stop")?;
    let s = stub(&[TERMINATE, STATUS])?;
    let out = run_wired(&["stop"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let line = stdout(&out);
    assert!(line.contains("terminated=com.x"), "{line}");
    let out = run_wired(&["status"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.captured()?;
    assert_eq!(captured.len(), 2, "{captured:?}");
    Ok(())
}

#[test]
fn screenshot_without_path_prints_base64() -> Result<(), Failure> {
    let home = tmp_home("shot-out")?;
    let s = stub(&[SCREENSHOT])?;
    let out = run_wired(&["screenshot"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("iVBORw0KGgo"), "{text}");
    Ok(())
}

#[test]
fn screenshot_with_path_writes_png_and_reports_bytes() -> Result<(), Failure> {
    let home = tmp_home("shot-file")?;
    let s = stub(&[SCREENSHOT])?;
    let path = home.join("shot.png");
    let path_str = path.to_string_lossy().into_owned();
    let out = run_wired(&["screenshot", &path_str], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let line = stdout(&out);
    assert!(line.contains("screenshot="), "{line}");
    assert!(line.contains("bytes="), "{line}");
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
    assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47], "PNG magic expected");
    Ok(())
}

#[test]
fn screenshot_json_stays_envelope() -> Result<(), Failure> {
    let home = tmp_home("shot-json")?;
    let s = stub(&[SCREENSHOT])?;
    let out = run_wired(&["screenshot", "--json"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(stdout(&out).trim())
        .map_err(|e| fail(&format!("json stdout: {e}")))?;
    assert_eq!(v["command"], "screenshot");
    assert!(v["data"]["png_base64"].as_str().is_some());
    Ok(())
}

fn commands_in_help() -> Result<BTreeSet<String>, Failure> {
    let home = tmp_home("help")?;
    let out = run(&["--help"], &home, &[])?;
    let text = stdout(&out);
    let mut names = BTreeSet::new();
    let mut in_commands = false;
    for line in text.lines() {
        if line.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if in_commands {
            let trimmed = line.trim();
            if trimmed.is_empty() || line.starts_with("Options:") {
                break;
            }
            if let Some(name) = trimmed.split_whitespace().next()
                && name != "help"
            {
                names.insert(name.to_owned());
            }
        }
    }
    Ok(names)
}

/// Command rows indent exactly two spaces; deeper-indented lines are
/// wrapped descriptions, not new names.
fn commands_in_skills() -> Result<BTreeSet<String>, Failure> {
    let home = tmp_home("skills")?;
    let out = run(&["skills"], &home, &[])?;
    let text = stdout(&out);
    let mut names = BTreeSet::new();
    let mut in_commands = false;
    for line in text.lines() {
        if line.starts_with("COMMANDS") {
            in_commands = true;
            continue;
        }
        if in_commands && (line.trim().is_empty() || !line.starts_with("  ")) {
            break;
        }
        if in_commands
            && line.starts_with("  ")
            && !line.starts_with("   ")
            && let Some(name) = line.split_whitespace().next()
        {
            names.insert(name.to_owned());
        }
    }
    Ok(names)
}

#[test]
fn skills_guide_matches_help_in_both_directions() -> Result<(), Failure> {
    let help = commands_in_help()?;
    let skills = commands_in_skills()?;
    for name in &help {
        assert!(
            skills.contains(name),
            "help lists {name} but skills omits it"
        );
    }
    for name in &skills {
        assert!(
            help.contains(name),
            "skills names {name} but no such command"
        );
    }
    Ok(())
}

#[test]
fn skills_guide_teaches_ref_lifecycle_and_recovery() -> Result<(), Failure> {
    let home = tmp_home("skills-body")?;
    let out = run(&["skills"], &home, &[])?;
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    for needle in [
        "STALE_REF",
        "AMBIGUOUS_TARGET",
        "re-snapshot",
        "settled=false",
        "tap <x> <y>",
        "activate",
        "AGENT_MOBILE_URL",
    ] {
        assert!(text.contains(needle), "skills missing {needle:?}");
    }
    Ok(())
}
