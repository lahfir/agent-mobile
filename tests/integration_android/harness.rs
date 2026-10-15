//! Android live-gate helpers: bounded adb, forward sets, session-entry
//! checks, console-token seeding, and bounded cleanup — so the test body
//! stays one flow plus one assert.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::process::run_bounded;
use agent_mobile_core::state::{SessionEntry, State, StateStore};

use super::common::{self, code, run, stderr, stdout};

const PKG: &str = "com.lahfir.agentmobile.driver";

pub fn device_key() -> String {
    std::env::var("AGENT_MOBILE_TEST_ANDROID_DEVICE")
        .unwrap_or_else(|_| super::DEVICE_KEY.to_owned())
}

pub fn adb_path() -> PathBuf {
    let root = std::env::var("ANDROID_SDK_ROOT")
        .or_else(|_| std::env::var("ANDROID_HOME"))
        .unwrap_or_default();
    PathBuf::from(root).join("platform-tools/adb")
}

/// Bounded checked adb: nonzero/timeout fails; stderr is control-cleaned.
pub fn adb_out(args: &[&str]) -> Result<String, Failure> {
    let out = run_bounded(
        std::process::Command::new(adb_path()).args(args),
        Duration::from_secs(20),
    )?;
    if !out.status.success() {
        let clean: String = String::from_utf8_lossy(&out.stderr)
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(400)
            .collect();
        return Err(common::fail(&format!("adb {args:?} failed: {clean}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn adb_scoped(serial: &str, args: &[&str]) -> Result<String, Failure> {
    let mut a = vec!["-s", serial];
    a.extend_from_slice(args);
    adb_out(&a)
}

/// `adb forward --list` rows as a set; the baseline may be nonempty.
pub fn forward_set() -> Result<BTreeSet<String>, Failure> {
    Ok(adb_out(&["forward", "--list"])?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Copy the invoking HOME's console auth token so `emu avd name`
/// correlation works under the isolated HOME; file stays mode 0600.
pub fn seed_console_token(home: &Path) -> Result<(), Failure> {
    let Some(real_home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let src = PathBuf::from(real_home).join(".emulator_console_auth_token");
    if !src.exists() {
        return Ok(());
    }
    let dst = home.join(".emulator_console_auth_token");
    std::fs::copy(&src, &dst).map_err(Failure::from)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o600))
            .map_err(Failure::from)?;
    }
    Ok(())
}

fn avd_home_from(
    avd_home: Option<&str>,
    user_home: Option<&str>,
    emulator_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    avd_home
        .map(PathBuf::from)
        .or_else(|| user_home.map(|p| PathBuf::from(p).join("avd")))
        .or_else(|| emulator_home.map(|p| PathBuf::from(p).join("avd")))
        .or_else(|| home.map(|p| PathBuf::from(p).join(".android/avd")))
}

fn inherited_avd_home() -> Result<PathBuf, Failure> {
    let env_path = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
    let avd = env_path("ANDROID_AVD_HOME");
    let user = env_path("ANDROID_USER_HOME");
    let emulator = env_path("ANDROID_EMULATOR_HOME");
    let home = env_path("HOME");
    avd_home_from(
        avd.as_deref(),
        user.as_deref(),
        emulator.as_deref(),
        home.as_deref(),
    )
    .ok_or_else(|| common::fail("no parent AVD home is available"))
}

pub fn cli(home: &Path, args: &[&str]) -> Result<std::process::Output, Failure> {
    let avd_home = inherited_avd_home()?;
    let avd_home = avd_home.to_string_lossy();
    run(
        args,
        &home.to_path_buf(),
        &[
            ("AGENT_MOBILE_REPO_ROOT", env!("CARGO_MANIFEST_DIR")),
            ("AGENT_MOBILE_BOOT_BUDGET_SECS", "600"),
            ("ANDROID_AVD_HOME", avd_home.as_ref()),
        ],
    )
}

/// Exit 0, protocol v1, `ok:true` — anything else is a gate failure.
pub fn cli_ok_json(home: &Path, args: &[&str]) -> Result<serde_json::Value, Failure> {
    let out = cli(home, args)?;
    if code(&out) != 0 {
        return Err(common::fail(&format!(
            "agent-mobile {args:?} exited {}: {}",
            code(&out),
            stderr(&out)
        )));
    }
    let env: serde_json::Value =
        serde_json::from_str(&stdout(&out)).map_err(|e| common::fail(&format!("bad json: {e}")))?;
    if env["version"] != "1" || env["ok"] != true {
        return Err(common::fail("envelope not ok/v1"));
    }
    Ok(env)
}

pub fn state(home: &Path) -> State {
    StateStore::at(&home.join(".agent-mobile")).load()
}

pub fn entry_for<'a>(state: &'a State, key: &str) -> Result<&'a SessionEntry, Failure> {
    state
        .devices
        .get(key)
        .ok_or_else(|| common::fail("session row missing"))
}

/// The recorded Android row must carry cleanup metadata, a live serve
/// pid, real artifact paths — and never a token value.
pub fn check_entry(home: &Path, key: &str) -> Result<u32, Failure> {
    let st = state(home);
    let e = entry_for(&st, key)?;
    if e.platform.as_deref() != Some("android")
        || e.device_id.as_deref() != Some("avd:agent-mobile-api37")
        || e.serial.is_none()
        || e.forward_port.is_none()
        || e.device_port.is_none()
        || e.bridge_port.is_none()
        || !agent_mobile_core::process::pid_alive(e.pid)
    {
        return Err(common::fail(
            "session row missing android metadata or live pid",
        ));
    }
    if e.apk_source.as_ref().is_none_or(|p| !Path::new(p).exists()) {
        return Err(common::fail("apk_source absent or missing file"));
    }
    if e.log_file.as_ref().is_none_or(|p| !Path::new(p).exists()) {
        return Err(common::fail("log_file absent or missing file"));
    }
    let raw = std::fs::read_to_string(home.join(".agent-mobile/state.json"))?;
    let doc: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| common::fail(&format!("state json parse: {e}")))?;
    if doc.pointer(&format!("/devices/{key}/token")).is_some() {
        return Err(common::fail("state carries a token field"));
    }
    Ok(e.pid)
}

/// Live forwards must equal baseline plus exactly the session's owned row.
pub fn check_forward(home: &Path, key: &str, baseline: &BTreeSet<String>) -> Result<(), Failure> {
    let st = state(home);
    let e = entry_for(&st, key)?;
    let mut expect = baseline.clone();
    expect.insert(format!(
        "{} tcp:{} tcp:{}",
        e.serial.clone().unwrap_or_default(),
        e.forward_port.unwrap_or_default(),
        e.device_port.unwrap_or_default(),
    ));
    if forward_set()? != expect {
        return Err(common::fail("forward set is not baseline + owned row"));
    }
    Ok(())
}

/// TERM only the recorded pid, then poll until row/token/pid are gone
/// and forwards equal baseline (sleeps live here, never in the test).
pub fn terminate_and_wait(
    home: &Path,
    key: &str,
    baseline: &BTreeSet<String>,
) -> Result<(), Failure> {
    let st = state(home);
    let Some(e) = st.devices.get(key) else {
        return Err(common::fail("no session row to terminate"));
    };
    let pid = e.pid;
    let token = e.token_file.clone();
    let _ = agent_mobile_core::process::terminate(pid);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let gone = !state(home).devices.contains_key(key)
            && !home.join(".agent-mobile/tokens").join(&token).exists()
            && !agent_mobile_core::process::pid_alive(pid)
            && forward_set().map(|f| f == *baseline).unwrap_or(false);
        if gone {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(common::fail("cleanup did not complete within 30s"))
}

/// Emulator stays `device`, driver APK stays installed, service stays on.
pub fn check_device_survives(serial: &str) -> Result<(), Failure> {
    let listed = adb_out(&["devices"])?;
    let alive = listed.lines().any(|l| {
        let mut f = l.split_whitespace();
        f.next() == Some(serial) && f.next() == Some("device")
    });
    if !alive {
        return Err(common::fail("emulator not in `device` state after cleanup"));
    }
    if !adb_scoped(serial, &["shell", "pm", "path", PKG])?.contains("package:") {
        return Err(common::fail("driver APK not installed after cleanup"));
    }
    let svc = adb_scoped(
        serial,
        &[
            "shell",
            "settings",
            "get",
            "secure",
            "enabled_accessibility_services",
        ],
    )?;
    let short = "com.lahfir.agentmobile.driver/.AgentMobileAccessibilityService";
    let long = "com.lahfir.agentmobile.driver/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService";
    if !(svc.contains(short) || svc.contains(long)) {
        return Err(common::fail("accessibility service disabled after cleanup"));
    }
    Ok(())
}

/// Bounded driver-log tail, gathered before teardown removes the HOME.
pub fn driver_log_tail(home: &Path) -> String {
    let Ok(entries) = std::fs::read_dir(home.join(".agent-mobile")) else {
        return "no driver log directory".to_owned();
    };
    let mut out = String::new();
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("driver-"))
        {
            continue;
        }
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        for line in body
            .lines()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() {
        "no driver log".to_owned()
    } else {
        out
    }
}

/// Failure-path sweeper: TERM every recorded serve pid and poll until
/// rows, token files, and pids are gone and forwards equal the baseline —
/// a mid-flow failure cannot leak a live serve while logs are collected.
pub fn cleanup_remaining(home: &Path, baseline: &BTreeSet<String>) -> Result<(), Failure> {
    live::cleanup_dead_rows(home)?;
    let st = state(home);
    let targets: Vec<(u32, String)> = st
        .devices
        .values()
        .map(|e| (e.pid, e.token_file.clone()))
        .collect();
    for (pid, _) in &targets {
        let _ = agent_mobile_core::process::terminate(*pid);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let st = state(home);
        let pids_dead = targets
            .iter()
            .all(|(p, _)| !agent_mobile_core::process::pid_alive(*p));
        let rows_gone = st.devices.is_empty();
        let tokens_gone = targets
            .iter()
            .all(|(_, t)| !home.join(".agent-mobile/tokens").join(t).exists());
        let forwards_clean = forward_set().map(|f| f == *baseline).unwrap_or(false);
        if pids_dead && rows_gone && tokens_gone && forwards_clean {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(common::fail(
        "cleanup_remaining timed out; resources may be live",
    ))
}

pub fn snapshot_id(v: &serde_json::Value) -> Result<String, Failure> {
    v["data"]["snapshot_id"]
        .as_str()
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| common::fail("snapshot_id empty"))
}

pub fn refs_count(v: &serde_json::Value) -> u64 {
    v["data"]["ref_count"].as_u64().unwrap_or(0)
}

#[path = "harness_live.rs"]
pub mod live;

pub use live::*;

#[cfg(test)]
mod tests {
    use super::avd_home_from;

    #[test]
    fn avd_home_precedence_matches_android_tooling() {
        assert_eq!(
            avd_home_from(Some("/a"), Some("/u"), Some("/e"), Some("/h")),
            Some("/a".into())
        );
        assert_eq!(
            avd_home_from(None, Some("/u"), Some("/e"), Some("/h")),
            Some("/u/avd".into())
        );
        assert_eq!(
            avd_home_from(None, None, Some("/e"), Some("/h")),
            Some("/e/avd".into())
        );
        assert_eq!(
            avd_home_from(None, None, None, Some("/h")),
            Some("/h/.android/avd".into())
        );
    }
}
