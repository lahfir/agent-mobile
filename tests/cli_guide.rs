//! Guide accuracy: `--help` and `skills` name every command the CLI runs,
//! so agents discover verbs from either surface.

/// Shared stub harness; this file uses only the unwired half of it.
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
