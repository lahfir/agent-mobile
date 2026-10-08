//! Discovery parsing and AVD correlation.

use std::path::PathBuf;

use agent_mobile_core::error::Failure;

use super::{AndroidDeviceKind, AndroidDeviceState, avd_name, discover, parse_devices};
use crate::adb::Adb;
use crate::testkit::{FakeRunner, output};

/// Every `getprop` probe must be serial-scoped, in row order.
fn assert_os_probes_scoped(calls: &[Vec<String>]) {
    let probes: Vec<_> = calls
        .iter()
        .filter(|c| c.iter().any(|a| a.contains("'getprop'")))
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
