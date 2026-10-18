//! The agent guide is a valid skill file, and per-verb help keeps the
//! Android system-UI return guidance.

#[allow(dead_code, reason = "shared harness; this suite uses the unwired half")]
mod common;

use agent_mobile_core::error::Failure;

use common::{code, run, stderr, stdout, tmp_home};

#[test]
fn guide_is_a_skill_file_named_after_its_directory() -> Result<(), Failure> {
    let home = tmp_home("skill-frontmatter")?;
    let out = run(&["skills"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    let front = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(front, _)| front)
        .unwrap_or_default();
    assert!(front.lines().any(|l| l == "name: agent-mobile"), "{front}");
    let description = front
        .lines()
        .find_map(|l| l.strip_prefix("description: "))
        .unwrap_or_default();
    assert!(
        (1..=1024).contains(&description.len()),
        "description must be 1-1024 chars: {description}"
    );
    Ok(())
}

#[test]
fn center_help_prescribes_back_for_android_return() -> Result<(), Failure> {
    let home = tmp_home("center-android-return")?;
    let out = run(&["center", "--help"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let help = stdout(&out);
    assert!(
        help.contains("`back` on Android"),
        "center help must name back as the Android return: {help}"
    );
    Ok(())
}
