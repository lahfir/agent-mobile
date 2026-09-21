//! Live-simulator gate (KTD15, R16): one real boot through lazy start, one
//! real snapshot, shape asserted, state torn down. Runs only when named —
//! CI invokes it explicitly on macOS with `-- --ignored`.

#[allow(dead_code, reason = "shared harness; this binary uses only part of it")]
mod common;

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::StateStore;

use common::{code, run, stderr, stdout, tmp_home};

/// End the session this test started: TERM the recorded serve pid forwards
/// to the runner, then drop the temp HOME entirely.
fn teardown(home: &std::path::Path) {
    let store = StateStore::at(&home.join(".agent-mobile"));
    for entry in store.load().devices.values() {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-TERM", &entry.pid.to_string()])
            .status();
    }
    let _ = std::fs::remove_dir_all(home);
}

/// From an empty HOME the first verb lazy-boots the default simulator,
/// records the session, and answers; a following snapshot returns the
/// contract header shape. Budget: the lazy path allows minutes for a cold
/// build; a warm checkout lands in seconds.
#[test]
#[ignore = "requires macOS with Xcode and a bootable iPhone simulator"]
fn first_verb_lazy_boots_and_snapshots() -> Result<(), Failure> {
    let home = tmp_home("integration")?;
    let out = run(&["status"], &home, &[])?;
    if code(&out) != 0 {
        let err = stderr(&out);
        teardown(&home);
        return Err(common::fail(&format!(
            "status after lazy boot failed: {err}"
        )));
    }
    let boot_progress = stderr(&out);
    assert!(
        boot_progress.contains("starting") || stdout(&out).contains("app="),
        "expected boot progress or an answer; stderr: {boot_progress}"
    );

    let out = run(&["snapshot"], &home, &[])?;
    let text = stdout(&out);
    if code(&out) != 0 {
        let err = stderr(&out);
        teardown(&home);
        return Err(common::fail(&format!("snapshot failed: {err}")));
    }
    let header = text.lines().next().unwrap_or_default();
    for needle in ["app=", "snapshot=@", "refs=", "settled=", "elapsed_ms="] {
        assert!(
            header.contains(needle),
            "snapshot header missing {needle:?}: {header}"
        );
    }

    let state = StateStore::at(&home.join(".agent-mobile")).load();
    assert!(!state.devices.is_empty(), "lazy boot must save the session");
    teardown(&home);
    assert!(
        !home.join(".agent-mobile/state.json").exists(),
        "teardown removed the session state"
    );
    Ok(())
}
