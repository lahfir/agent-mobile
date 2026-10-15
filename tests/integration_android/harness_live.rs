use std::path::Path;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::process::run_bounded;
use agent_mobile_core::state::{SessionEntry, StateStore};

use super::common;
use super::{adb_path, adb_scoped, entry_for, forward_set, state};

/// The live session's token must be exactly 43 bytes on disk and absent
/// from every observable surface — state JSON, driver log, and the serve
/// process's own argv/env when readable. Never echoes the token.
pub fn check_token_hygiene(home: &Path, key: &str) -> Result<(), Failure> {
    let st = state(home);
    let e = entry_for(&st, key)?;
    let token_path = home.join(".agent-mobile/tokens").join(&e.token_file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&token_path)
            .map_err(Failure::from)?
            .permissions()
            .mode();
        if mode & 0o777 != 0o600 {
            return Err(common::fail("token file is not mode 0600"));
        }
    }
    let token = std::fs::read_to_string(&token_path).map_err(Failure::from)?;
    if token.len() != 43 {
        return Err(common::fail("token file is not 43 bytes"));
    }
    let mut surfaces = vec![std::fs::read_to_string(
        home.join(".agent-mobile/state.json"),
    )?];
    if let Some(log) = e.log_file.as_deref() {
        surfaces.push(std::fs::read_to_string(log).unwrap_or_default());
    }
    for proc in ["cmdline", "environ"] {
        let p = format!("/proc/{}/{proc}", e.pid);
        if Path::new(&p).exists() {
            surfaces.push(std::fs::read_to_string(&p).unwrap_or_default());
        }
    }
    for (i, surface) in surfaces.iter().enumerate() {
        if surface.contains(&token) {
            return Err(common::fail(&format!("token leaked into surface {i}")));
        }
    }
    Ok(())
}

/// `run-as` with an ordinary app UID must be denied by the provider's
/// shell/root gate — `Permission Denial` or `SecurityException`, never a
/// bundle — proving the manifest permission + caller check are real.
pub fn check_provider_denial(serial: &str) -> Result<(), Failure> {
    let args = vec![
        "shell",
        "run-as",
        "com.lahfir.agentmobile.driver",
        "content",
        "call",
        "--uri",
        "content://com.lahfir.agentmobile.driver.provision",
        "--method",
        "provision",
    ];
    let mut a = vec!["-s", serial];
    a.extend_from_slice(&args);
    let out = run_bounded(
        std::process::Command::new(adb_path()).args(&a),
        Duration::from_secs(20),
    )?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if text.contains("Bundle[")
        || !(text.contains("Permission Denial") || text.contains("SecurityException"))
    {
        return Err(common::fail(
            "run-as provision was not rejected by the provider",
        ));
    }
    Ok(())
}

/// Depth-first walk of `tree` for the first node whose
/// `available_actions` contains exactly `Tap` — returns its `ref_id`.
pub fn first_tap_ref(tree: &serde_json::Value) -> Option<String> {
    if tree["available_actions"]
        .as_array()
        .is_some_and(|a| a.iter().any(|x| x == "Tap"))
        && let Some(r) = tree["ref_id"].as_str()
    {
        return Some(r.to_owned());
    }
    tree["children"].as_array()?.iter().find_map(first_tap_ref)
}

/// KILL the recorded serve pid, wait for death, and confirm the stale
/// row + its exact owned forward survive so the next call must reclaim.
pub fn sigkill_and_await_stale(home: &Path, key: &str) -> Result<u32, Failure> {
    let st = state(home);
    let e = entry_for(&st, key)?.clone();
    let pid = e.pid;
    let out = run_bounded(
        std::process::Command::new("/bin/kill").args(["-KILL", &pid.to_string()]),
        Duration::from_secs(5),
    )?;
    if !out.status.success() {
        return Err(common::fail("kill -KILL failed for the recorded pid"));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while agent_mobile_core::process::pid_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    if agent_mobile_core::process::pid_alive(pid) {
        return Err(common::fail("killed serve pid still alive"));
    }
    let st2 = state(home);
    if !st2.devices.contains_key(key) {
        return Err(common::fail("stale row vanished without reclaim"));
    }
    let rows = forward_set()?;
    let owned = format!(
        "{} tcp:{} tcp:{}",
        e.serial.clone().unwrap_or_default(),
        e.forward_port.unwrap_or_default(),
        e.device_port.unwrap_or_default(),
    );
    if !rows.contains(&owned) {
        return Err(common::fail("owned forward missing before reclaim"));
    }
    Ok(pid)
}

pub(super) fn cleanup_dead_rows(home: &Path) -> Result<(), Failure> {
    let st = state(home);
    let store = StateStore::at(&home.join(".agent-mobile"));
    for (key, entry) in st
        .devices
        .iter()
        .filter(|(_, entry)| !agent_mobile_core::process::pid_alive(entry.pid))
    {
        if entry.platform.as_deref() == Some("android") {
            cleanup_dead_forward(entry)?;
        }
        store.remove_token(&entry.token_file)?;
        store.remove(key)?;
    }
    Ok(())
}

fn cleanup_dead_forward(entry: &SessionEntry) -> Result<(), Failure> {
    let (Some(serial), Some(local), Some(remote)) = (
        entry.serial.as_deref(),
        entry.forward_port,
        entry.device_port,
    ) else {
        return Err(common::fail("dead Android row lacks forward ownership"));
    };
    let owned = format!("{serial} tcp:{local} tcp:{remote}");
    if !forward_set()?.contains(&owned) {
        return Ok(());
    }
    let removed = adb_scoped(serial, &["forward", "--remove", &format!("tcp:{local}")]);
    if !forward_set()?.contains(&owned) {
        return Ok(());
    }
    match removed {
        Err(error) => Err(error),
        Ok(_) => Err(common::fail("dead session forward survived cleanup")),
    }
}

#[cfg(test)]
mod tests {
    use super::first_tap_ref;

    #[test]
    fn first_tap_ref_selects_tap_and_skips_click_only() {
        let tree = serde_json::json!({
            "available_actions": ["Click"],
            "ref_id": "@aa:e1",
            "children": [
                {"available_actions": ["Click"], "ref_id": "@aa:e2"},
                {
                    "available_actions": ["Click", "Tap"],
                    "ref_id": "@aa:e3",
                    "children": [{"available_actions": ["Tap"], "ref_id": "@aa:e4"}],
                }
            ]
        });
        assert_eq!(
            first_tap_ref(&tree).as_deref(),
            Some("@aa:e3"),
            "depth-first Tap match wins; Click-only nodes are skipped"
        );
    }
}
