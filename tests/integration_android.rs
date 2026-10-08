//! Live Android gate (U8): concurrent lazy `status` proves boot-lock
//! serialization, real bridge verbs exercise the session, then TERM
//! proves owned-resource cleanup — twice. Ignored: needs a booted
//! authorized emulator plus a built driver APK.

#[allow(dead_code, reason = "shared harness; this binary uses only part of it")]
mod common;

use std::collections::BTreeSet;
use std::path::Path;

use agent_mobile_core::error::Failure;

use common::{code, stderr, stdout, tmp_home};
use harness::{
    check_device_survives, check_entry, check_forward, check_provider_denial, check_token_hygiene,
    cleanup_remaining, cli, cli_ok_json, device_key, driver_log_tail, entry_for, first_tap_ref,
    forward_set, refs_count, seed_console_token, sigkill_and_await_stale, snapshot_id, state,
    terminate_and_wait,
};

#[path = "integration_android/harness.rs"]
mod harness;

const DEVICE_KEY: &str = "android:avd:agent-mobile-api37";

/// The whole live flow; the `#[test]` only asserts on its Result after
/// cleanup so diagnostics and owned-resource teardown always run.
fn flow(home: &Path, key: &str, baseline: &BTreeSet<String>) -> Result<(), Failure> {
    seed_console_token(home)?;
    let dev = format!("--device={key}");
    let t1 = std::thread::spawn({
        let (home, dev) = (home.to_path_buf(), dev.clone());
        move || cli(&home, &[&dev, "--json", "status"])
    });
    let t2 = std::thread::spawn({
        let (home, dev2) = (home.to_path_buf(), dev.clone());
        move || cli(&home, &[&dev2, "--json", "status"])
    });
    let o1 = t1.join().map_err(|_| common::fail("spawn 1 panicked"))??;
    let o2 = t2.join().map_err(|_| common::fail("spawn 2 panicked"))??;
    for out in [&o1, &o2] {
        if code(out) != 0 {
            return Err(common::fail(&format!(
                "concurrent status failed: {}",
                stderr(out)
            )));
        }
        let env: serde_json::Value =
            serde_json::from_str(&stdout(out)).map_err(|e| common::fail(&format!("{e}")))?;
        if env["version"] != "1" || env["ok"] != true {
            return Err(common::fail("concurrent status not ok/v1"));
        }
    }
    if state(home).devices.keys().count() != 1 {
        return Err(common::fail("expected exactly one session row"));
    }
    let pid1 = check_entry(home, key)?;
    check_forward(home, key, baseline)?;
    check_token_hygiene(home, key)?;
    let serial = entry_for(&state(home), key)?
        .serial
        .clone()
        .unwrap_or_default();
    check_provider_denial(&serial)?;
    let snap_id = verbs_and_refs(home, &dev)?;
    let home_out = cli_ok_json(home, &[&dev, "--json", "home"])?;
    let home_id = snapshot_id(&home_out)?;
    if home_id == snap_id {
        return Err(common::fail("home did not mint a fresh snapshot"));
    }
    let png = home.join("android-ci.png");
    let png_s = png.display().to_string();
    let out = cli(home, &[&dev, "screenshot", &png_s])?;
    if code(&out) != 0 {
        return Err(common::fail(&format!(
            "screenshot failed: {}",
            stderr(&out)
        )));
    }
    let bytes = std::fs::read(&png)?;
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(common::fail("screenshot is not a PNG"));
    }
    terminate_and_wait(home, key, baseline)?;
    check_device_survives(&serial)?;
    let re = cli_ok_json(home, &[&dev, "--json", "status"])?;
    if re["ok"] != true {
        return Err(common::fail("re-serve status not ok"));
    }
    let pid2 = check_entry(home, key)?;
    if pid2 == pid1 {
        return Err(common::fail("re-serve reused the dead pid"));
    }
    check_forward(home, key, baseline)?;
    let killed = sigkill_and_await_stale(home, key)?;
    let re2 = cli_ok_json(home, &[&dev, "--json", "status"])?;
    if re2["ok"] != true {
        return Err(common::fail("post-SIGKILL status not ok"));
    }
    let pid3 = check_entry(home, key)?;
    if pid3 == pid2 || pid3 == killed {
        return Err(common::fail("stale reclaim did not start a new serve"));
    }
    check_forward(home, key, baseline)?;
    terminate_and_wait(home, key, baseline)?;
    check_device_survives(&serial)?;
    Ok(())
}

/// Verb surface on the live session: launch → snapshot → ref tap →
/// `STALE_REF` → home; returns the last fresh snapshot id for the caller's
/// downstream comparisons.
fn verbs_and_refs(home: &Path, dev: &str) -> Result<String, Failure> {
    let launch = cli_ok_json(
        home,
        &[dev, "--json", "launch", "com.google.android.deskclock"],
    )?;
    if launch["data"]["app"] != "com.google.android.deskclock" || refs_count(&launch) == 0 {
        return Err(common::fail("launch returned no deskclock snapshot"));
    }
    let launch_id = snapshot_id(&launch)?;
    let snap = cli_ok_json(home, &[dev, "--json", "snapshot"])?;
    let mut snap_id = snapshot_id(&snap)?;
    if snap_id == launch_id || refs_count(&snap) == 0 || !snap["data"]["tree"].is_object() {
        return Err(common::fail("snapshot not fresh or empty"));
    }
    let Some(tap_ref) = first_tap_ref(&snap["data"]["tree"]) else {
        return Err(common::fail("no tappable ref in deskclock snapshot"));
    };
    let tap = cli_ok_json(home, &[dev, "--json", "tap", &tap_ref])?;
    if snapshot_id(&tap)? == snap_id {
        return Err(common::fail("tap did not mint a fresh snapshot"));
    }
    let stale = cli(home, &[dev, "--json", "tap", &tap_ref])?;
    if code(&stale) == 0 {
        return Err(common::fail("stale ref accepted"));
    }
    let stale_env: serde_json::Value =
        serde_json::from_str(&stdout(&stale)).map_err(|e| common::fail(&format!("{e}")))?;
    if stale_env["error"]["code"] != "STALE_REF" {
        return Err(common::fail("reused ref did not fail STALE_REF"));
    }
    snap_id = snapshot_id(&cli_ok_json(home, &[dev, "--json", "snapshot"])?)?;
    Ok(snap_id)
}

#[test]
#[ignore = "requires an authorized, booted Android AVD and built driver APK"]
fn public_cli_lazy_android_session_cleans_and_restarts() {
    let key = device_key();
    let result = (|| -> Result<(), Failure> {
        let home = tmp_home("integration-android")?;
        let baseline = forward_set().inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&home);
        })?;
        let flow_result = flow(&home, &key, &baseline);
        let log = driver_log_tail(&home);
        let cleanup_result = cleanup_remaining(&home, &baseline);
        match (flow_result, cleanup_result) {
            (Ok(()), Ok(())) => std::fs::remove_dir_all(&home).map_err(Failure::from),
            (Err(f), Ok(())) => match std::fs::remove_dir_all(&home) {
                Ok(()) => Err(Failure::local(f.message().to_owned(), log)),
                Err(e) => Err(Failure::local(
                    format!("session cleanup ok but temp home removal failed: {e}"),
                    log,
                )),
            },
            (Ok(()), Err(c)) => Err(Failure::local(c.message().to_owned(), log)),
            (Err(f), Err(c)) => Err(Failure::local(
                format!("{}; cleanup also failed: {}", f.message(), c.message()),
                log,
            )),
        }
    })();
    assert!(result.is_ok(), "android live gate failed: {result:?}");
}
