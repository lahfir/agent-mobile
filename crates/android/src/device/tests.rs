//! Discovery parsing, AVD correlation, and bounded boot.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use agent_mobile_core::error::Failure;

use super::{AndroidDeviceKind, AndroidDeviceState, avd_name, discover, parse_devices};
use crate::adb::Adb;
use crate::boot::{boot_avd, spawn_detached};
use crate::testkit::{FakeRunner, output};

/// Every `getprop` probe must be serial-scoped, in row order.
fn assert_os_probes_scoped(calls: &[Vec<String>]) {
    let probes: Vec<_> = calls
        .iter()
        .filter(|c| c.iter().any(|a| a == "getprop"))
        .collect();
    assert_eq!(probes.len(), 3);
    for (probe, serial) in probes.iter().zip([
        "emulator-5554".to_owned(),
        "0a1b2c3d4e".to_owned(),
        "192.168.1.9:5555".to_owned(),
    ]) {
        assert_eq!(probe[1], serial, "getprop must scope the row's serial");
    }
}

const LIST: &str = "List of devices attached\n\
emulator-5554          device product:sdk_gphone64 model:emu64a device:emu64a transport_id:1\n\
0a1b2c3d4e             device usb:1-2 product:redfin model:Pixel_5 device:redfin\n\
192.168.1.9:5555       device product:wifi model:Pixel_6\n\
deadbeef               offline\n\
cafe0011               unauthorized\n";

#[test]
fn parses_every_state_and_kind() {
    let rows = parse_devices(LIST);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].serial, "emulator-5554");
    assert_eq!(rows[0].state, "device");
    assert_eq!(rows[0].fields.get("model"), Some(&"emu64a".to_owned()));
    assert_eq!(rows[3].serial, "deadbeef");
    assert_eq!(rows[3].state, "offline");
    assert_eq!(rows[4].state, "unauthorized");
}

#[test]
fn discover_classifies_and_correlates() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, LIST, ""),
        output(true, "agent-mobile-api37\nPixel_8_api37\n", ""),
        output(true, "agent-mobile-api37\nOK\n", ""),
        output(true, "16\n", ""),
        output(true, "15\n", ""),
        output(false, "", "boom"),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let scan = discover(&adb, PathBuf::from("emulator").as_path())?;
    assert_eq!(scan.targets.len(), 6);
    assert_eq!(scan.targets[0].os.as_deref(), Some("16"));
    assert_eq!(scan.targets[1].os.as_deref(), Some("15"));
    assert_eq!(scan.targets[2].os, None);
    assert_os_probes_scoped(&runner.calls());
    assert_target_shapes(&scan.targets);
    Ok(())
}

/// The LIST fixture must classify and correlate exactly.
fn assert_target_shapes(targets: &[super::AndroidTarget]) {
    let emu = &targets[0];
    assert_eq!(emu.id, "avd:agent-mobile-api37");
    assert_eq!(emu.kind, AndroidDeviceKind::Emulator);
    assert_eq!(emu.state, AndroidDeviceState::Device);
    assert_eq!(emu.avd.as_deref(), Some("agent-mobile-api37"));
    assert_eq!(targets[1].id, "0a1b2c3d4e");
    assert_eq!(targets[1].kind, AndroidDeviceKind::Usb);
    assert_eq!(targets[2].kind, AndroidDeviceKind::Wireless);
    assert_eq!(targets[3].state, AndroidDeviceState::Offline);
    assert_eq!(targets[4].state, AndroidDeviceState::Unauthorized);
}

#[test]
fn shutdown_avd_listed_once_not_duplicated() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, LIST, ""),
        output(true, "agent-mobile-api37\nPixel_8_api37\n", ""),
        output(true, "agent-mobile-api37\nOK\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let scan = discover(&adb, PathBuf::from("emulator").as_path())?;
    let avd_rows: Vec<_> = scan
        .targets
        .iter()
        .filter(|t| t.id == "avd:agent-mobile-api37")
        .collect();
    assert_eq!(avd_rows.len(), 1);
    let cold = scan.targets.iter().find(|t| t.id == "avd:Pixel_8_api37");
    let cold = cold.ok_or_else(|| Failure::local("missing cold avd", "fail"))?;
    assert_eq!(cold.serial, None);
    Ok(())
}

#[test]
fn uncorrelated_emulator_keeps_serial_id() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "agent-mobile-api37\n", ""),
        output(true, "other-avd\nOK\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let scan = discover(&adb, PathBuf::from("emulator").as_path())?;
    let emu = &scan.targets[0];
    assert_eq!(emu.id, "emulator-5554");
    assert!(
        scan.targets
            .iter()
            .any(|t| t.id == "avd:agent-mobile-api37")
    );
    Ok(())
}

#[test]
fn avd_name_strips_ok_suffix() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![output(true, "agent-mobile-api37\nOK\n", "")]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    assert_eq!(
        avd_name(&adb, "emulator-5554")?.as_deref(),
        Some("agent-mobile-api37")
    );
    Ok(())
}

#[test]
fn boot_rejects_unsafe_and_unknown_names() {
    let runner = FakeRunner::scripted(vec![output(true, "real-avd\n", "")]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    for bad in ["bad;name", "../up", "with space", "-flag"] {
        assert!(
            boot_avd(
                &adb,
                &PathBuf::from("emulator"),
                bad,
                PathBuf::from("/tmp/x").as_path(),
                std::time::Duration::from_secs(1)
            )
            .is_err()
        );
    }
    assert!(
        boot_avd(
            &adb,
            &PathBuf::from("emulator"),
            "not-listed",
            PathBuf::from("/tmp/x").as_path(),
            std::time::Duration::from_secs(1)
        )
        .is_err()
    );
    assert_eq!(runner.calls().len(), 1);
}

#[test]
fn boot_reuses_running_correlated_avd() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\nOK\n", ""),
        output(true, "1\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let booted = boot_avd(
        &adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/am-test-boot.log").as_path(),
        std::time::Duration::from_secs(1),
    )?;
    assert_eq!(booted.serial, "emulator-5554");
    assert!(
        runner
            .calls()
            .iter()
            .all(|c| c.first().is_some_and(|a| a != "-avd"))
    );
    Ok(())
}

#[test]
fn spawn_detached_records_pid_and_log() -> Result<(), Failure> {
    let dir = std::env::temp_dir().join(format!("am-spawn-{}", std::process::id()));
    let log = dir.join("emu.log");
    let pid = spawn_detached(Path::new("/usr/bin/true"), &[], &log)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log)
            .map_err(Failure::from)?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "log must be 0600, got {mode:o}");
    }
    assert!(pid > 0);
    assert!(log.exists());
    Ok(())
}

#[test]
fn boot_waits_for_boot_completed_on_running_avd() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\nOK\n", ""),
        output(true, "0", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\nOK\n", ""),
        output(true, "1", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let booted = boot_avd(
        &adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/x").as_path(),
        std::time::Duration::from_secs(5),
    )?;
    assert_eq!(booted.serial, "emulator-5554");
    assert_eq!(booted.pid, 0);
    assert_eq!(runner.calls().len(), 7);
    Ok(())
}

#[test]
fn boot_fails_for_unauthorized_running_row() {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tunauthorized\n", ""),
        output(true, "real-avd\nOK\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let err = boot_avd(
        &adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/x").as_path(),
        std::time::Duration::from_secs(5),
    )
    .err()
    .map(|e| format!("{} {}", e.message(), e.render()))
    .unwrap_or_default();
    assert!(err.contains("unauthorized"), "missing remedy: {err}");
}

#[test]
fn boot_fails_when_devices_probe_fails() {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(false, "", "daemon refused"),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    assert!(
        boot_avd(
            &adb,
            &PathBuf::from("emulator"),
            "real-avd",
            PathBuf::from("/tmp/x").as_path(),
            std::time::Duration::from_secs(5),
        )
        .is_err()
    );
    assert_eq!(runner.calls().len(), 2);
}

#[test]
fn boot_avd_until_cancelled_errors_before_next_probe() -> Result<(), Failure> {
    let cancelled = AtomicBool::new(true);
    let runner = FakeRunner::scripted(vec![
        output(true, "agent-mobile-api37\n", ""),
        output(
            true,
            "emulator-5554\tdevice product:x model:y device:z transport_id:1\n",
            "",
        ),
        output(true, "agent-mobile-api37\nOK\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let log = std::env::temp_dir().join("am-cancel-test.log");
    let err = crate::boot::boot_avd_until(
        &adb,
        PathBuf::from("emulator").as_path(),
        "agent-mobile-api37",
        &log,
        Duration::from_secs(30),
        &cancelled,
    )
    .err()
    .ok_or_else(|| Failure::local("cancel did not error", "fail"))?;
    assert!(format!("{err:?}").contains("cancelled"), "{err:?}");
    Ok(())
}

#[test]
fn precancelled_boot_never_spawns_avd() {
    let cancelled = AtomicBool::new(true);
    let runner = FakeRunner::scripted(vec![
        output(true, "agent-mobile-api37\n", ""),
        output(true, "List of devices attached\n\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let log = std::env::temp_dir().join("am-precancel-test.log");
    let err = crate::boot::boot_avd_until(
        &adb,
        PathBuf::from("emulator").as_path(),
        "agent-mobile-api37",
        &log,
        Duration::from_secs(30),
        &cancelled,
    );
    assert!(err.is_err());
    assert!(
        !runner.calls().iter().any(|c| c.iter().any(|a| a == "-avd")),
        "no emulator -avd command may issue when pre-cancelled"
    );
}
