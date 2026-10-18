//! CLI coverage for releasing a recorded driver session.

#[allow(dead_code, reason = "shared CLI and driver fixtures")]
mod common;

use std::process::{Command, Stdio};
use std::thread;

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

#[test]
fn release_terminates_the_recorded_serve_and_clears_its_row() -> Result<(), Failure> {
    let home = common::tmp_home("release")?;
    let root = home.join(".agent-mobile");
    std::fs::create_dir_all(&root)?;
    let store = StateStore::at(&root);
    let mut child = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut entry = SessionEntry::new(
        "http://127.0.0.1:1".to_owned(),
        child.id(),
        "fixture".to_owned(),
    );
    entry.process_started_at = agent_mobile_core::process::process_identity(child.id());
    store.upsert("ios:A", &entry)?;
    let watcher_store = store.clone();
    let watcher = thread::spawn(move || {
        let _ = child.wait();
        let _ = watcher_store.remove("ios:A");
    });
    let out = common::run(&["release"], &home, &[])?;
    assert_eq!(common::code(&out), 0, "{}", common::stderr(&out));
    assert!(common::stdout(&out).contains("released ios:A"));
    watcher
        .join()
        .map_err(|_| common::fail("release watcher"))?;
    assert!(store.entry("ios:A").is_none());
    Ok(())
}

#[test]
fn release_resolves_a_live_display_name() -> Result<(), Failure> {
    let home = common::tmp_home("release-display-name")?;
    let root = home.join(".agent-mobile");
    std::fs::create_dir_all(&root)?;
    let store = StateStore::at(&root);
    let mut child = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut entry = SessionEntry::new(
        "http://127.0.0.1:1".to_owned(),
        child.id(),
        "fixture".to_owned(),
    );
    entry.device_name = Some("iPhone 17".to_owned());
    entry.process_started_at = agent_mobile_core::process::process_identity(child.id());
    store.upsert("ios:A", &entry)?;
    let watcher_store = store.clone();
    let watcher = thread::spawn(move || {
        let _ = child.wait();
        let _ = watcher_store.remove("ios:A");
    });
    let out = common::run(&["release", "--device", "iPhone 17"], &home, &[])?;
    assert_eq!(common::code(&out), 0, "{}", common::stderr(&out));
    assert!(common::stdout(&out).contains("released ios:A"));
    watcher
        .join()
        .map_err(|_| common::fail("release watcher"))?;
    assert!(store.entry("ios:A").is_none());
    Ok(())
}

#[test]
fn release_without_a_live_session_succeeds() -> Result<(), Failure> {
    let home = common::tmp_home("release-empty")?;
    let out = common::run(&["release"], &home, &[])?;
    assert_eq!(common::code(&out), 0, "{}", common::stderr(&out));
    assert!(common::stdout(&out).contains("no driver running"));
    Ok(())
}

#[test]
fn release_does_not_signal_a_stale_identity() -> Result<(), Failure> {
    let home = common::tmp_home("release-stale")?;
    let root = home.join(".agent-mobile");
    std::fs::create_dir_all(&root)?;
    let store = StateStore::at(&root);
    let entry = SessionEntry::new(
        "http://127.0.0.1:1".to_owned(),
        std::process::id(),
        "fixture".to_owned(),
    );
    let mut stale = entry;
    stale.process_started_at = Some("not-this-process".to_owned());
    store.upsert("ios:A", &stale)?;
    let out = common::run(&["release", "--device", "ios:A"], &home, &[])?;
    assert_eq!(common::code(&out), 0, "{}", common::stderr(&out));
    assert!(common::stdout(&out).contains("stale driver row for ios:A"));
    assert!(store.entry("ios:A").is_some());
    Ok(())
}
