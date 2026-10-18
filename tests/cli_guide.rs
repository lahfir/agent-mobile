//! Static guide sections and Android system-UI return guidance stay accurate.

#[allow(dead_code, reason = "shared harness; this suite uses the unwired half")]
mod common;

use agent_mobile_core::error::Failure;

use common::{code, run, stderr, stdout, tmp_home};

/// Full text of one guide command row: the two-space row plus its
/// deeper-indented continuations, joined with spaces.
fn row_in_skills(text: &str, verb: &str) -> String {
    let mut row = String::new();
    let mut in_row = false;
    let mut in_commands = false;
    for line in text.lines() {
        if line.starts_with("COMMANDS") {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if line.trim().is_empty() || !line.starts_with("  ") {
            break;
        }
        if line.starts_with("  ") && !line.starts_with("   ") {
            if in_row {
                break;
            }
            if line.split_whitespace().next() == Some(verb) {
                in_row = true;
            } else {
                continue;
            }
        }
        if in_row {
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(line.trim());
        }
    }
    row
}

#[test]
fn guide_documents_repo_root_env() -> Result<(), Failure> {
    let home = tmp_home("repo-root-env")?;
    let out = run(&["skills"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("AGENT_MOBILE_REPO_ROOT"),
        "guide must document the driver-checkout locator"
    );
    Ok(())
}

#[test]
fn guide_documents_android_env() -> Result<(), Failure> {
    let home = tmp_home("android-env")?;
    let out = run(&["skills"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    for var in ["ANDROID_HOME", "AGENT_MOBILE_BOOT_BUDGET_SECS"] {
        assert!(text.contains(var), "guide must document {var}");
    }
    Ok(())
}

#[test]
fn center_row_prescribes_back_for_android_return() -> Result<(), Failure> {
    let home = tmp_home("center-android-return")?;
    let out = run(&["skills"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let row = row_in_skills(&stdout(&out), "center");
    assert!(!row.is_empty(), "guide omits the center row");
    assert!(
        row.contains("back"),
        "center row must name back as the Android return: {row}"
    );
    assert!(
        !row.contains("--app to return"),
        "center row must not prescribe snapshot --app to return on Android: {row}"
    );
    Ok(())
}
