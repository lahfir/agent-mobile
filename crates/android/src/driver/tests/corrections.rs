use std::path::PathBuf;

use agent_mobile_core::error::Failure;

use super::{SERVICE_COMPONENT, adb_with, enable_service, ensure_apk, output};

/// Temp driver dir, removed even on panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(tag: &str) -> TestDir {
    TestDir(std::env::temp_dir().join(format!(
        "am-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    )))
}

#[test]
fn enable_quotes_shell_syntax_in_existing_settings_value() -> Result<(), Failure> {
    let existing = "a.b/.C:x'$(rm -rf)'/.Evil";
    let merged = format!("{existing}:{SERVICE_COMPONENT}");
    let replies = vec![
        output(true, existing, ""),
        output(true, "1", ""),
        output(true, "1", ""),
        output(true, existing, ""),
        output(true, "", ""),
        output(true, &merged, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
    ];
    let (adb, runner) = adb_with(replies);
    enable_service(&adb, "s1")?;
    let calls = runner.calls();
    let remote_words: Vec<&String> = calls
        .iter()
        .flat_map(|c| c.iter())
        .filter(|a| a.contains("'put'"))
        .collect();
    assert_eq!(remote_words.len(), 1, "exactly one settings put runs");
    let remote = remote_words[0];
    assert!(
        remote.contains(&format!("'{}'", merged.replace('\'', "'\"'\"'"))),
        "the merged value is one quoted remote word: {remote}"
    );
    Ok(())
}

#[test]
fn stale_apk_still_runs_gradle_for_freshness() -> Result<(), Failure> {
    let tmp = temp_dir("apk-fresh");
    let dir = tmp.0.clone();
    let apk = dir.join("app/build/outputs/apk/debug/app-debug.apk");
    std::fs::create_dir_all(apk.parent().unwrap_or(&dir)).map_err(Failure::from)?;
    std::fs::write(&apk, b"apk").map_err(Failure::from)?;
    std::fs::write(dir.join("gradlew"), b"#!/bin/sh\nexit 0\n").map_err(Failure::from)?;
    let (adb, runner) = adb_with(vec![output(true, "", "")]);
    let got = ensure_apk(None, &dir, &adb)?;
    assert_eq!(got, apk);
    let calls = runner.calls();
    let builds: Vec<_> = calls
        .iter()
        .filter(|c| c.iter().any(|a| a == ":app:assembleDebug"))
        .collect();
    assert_eq!(builds.len(), 1, "one Gradle assemble runs: {calls:?}");
    assert!(builds[0].contains(&"--no-daemon".to_owned()));
    drop(tmp);
    Ok(())
}
