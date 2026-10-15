//! Guide accuracy: `--help` and `skills` name every command the CLI runs,
//! so agents discover verbs from either surface.

#[allow(dead_code, reason = "shared harness; this suite uses the unwired half")]
mod common;

use std::collections::BTreeSet;

use agent_mobile_core::error::Failure;

use common::{code, run, stderr, stdout, tmp_home};

/// The six verbs this file covers.
const GESTURE_VERBS: [&str; 6] = ["doubletap", "pinch", "hold", "back", "twofinger", "center"];

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
