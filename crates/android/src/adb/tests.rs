//! `adb` seam: serial scoping, owned output, tool resolution order.

use std::path::PathBuf;

use agent_mobile_core::error::Failure;

use super::{Adb, clean_output, resolve_sdk};
use crate::testkit::{FakeRunner, output};

#[test]
fn scoped_commands_prefix_serial() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![output(true, "done", "")]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    assert_eq!(
        adb.scoped("emulator-5554", &["shell", "id"])?.stdout,
        "done"
    );
    let calls = runner.calls();
    assert_eq!(calls[0], ["-s", "emulator-5554", "shell", "id"]);
    Ok(())
}

#[test]
fn second_serial_gets_its_own_scope() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    adb.scoped("s-a", &["get-state"])?;
    adb.scoped("s-b", &["get-state"])?;
    let calls = runner.calls();
    assert_eq!(calls[0][1], "s-a");
    assert_eq!(calls[1][1], "s-b");
    Ok(())
}

#[test]
fn unscoped_command_carries_no_serial() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    adb.unscoped(&["devices", "-l"])?;
    assert_eq!(runner.calls()[0], ["devices", "-l"]);
    Ok(())
}

#[test]
fn output_is_owned_and_control_cleaned() {
    let cleaned = clean_output(b"ok\x07\x08tail\n");
    assert_eq!(cleaned, "oktail\n");
    assert_eq!(clean_output(b"abc\x00def"), "abcdef");
}

#[test]
fn resolve_sdk_prefers_android_home() {
    let sdk = resolve_sdk(
        |key| {
            if key == "ANDROID_HOME" {
                Some("/opt/sdk".to_owned())
            } else {
                None
            }
        },
        |path| path.starts_with("/opt/sdk"),
    );
    assert_eq!(sdk.adb, PathBuf::from("/opt/sdk/platform-tools/adb"));
    assert_eq!(sdk.emulator, PathBuf::from("/opt/sdk/emulator/emulator"));
}

#[test]
fn resolve_sdk_falls_back_to_sdk_root_then_path() {
    let sdk = resolve_sdk(
        |key| {
            if key == "ANDROID_SDK_ROOT" {
                Some("/x/sdk".to_owned())
            } else {
                None
            }
        },
        |_| true,
    );
    assert_eq!(sdk.adb, PathBuf::from("/x/sdk/platform-tools/adb"));
    let none = resolve_sdk(|_| None, |_| false);
    assert_eq!(none.adb, PathBuf::from("adb"));
}

#[test]
fn command_output_debug_never_leaks_content() {
    let sentinel = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
    let out = output(
        true,
        &format!("Bundle[{{token={sentinel}}}]"),
        "stderr-junk",
    );
    let dbg = format!("{out:?}");
    assert!(!dbg.contains(sentinel), "debug leaked token: {dbg}");
    assert!(!dbg.contains("stderr-junk"), "debug leaked stderr: {dbg}");
    assert!(dbg.contains("stdout_len"), "missing len fields: {dbg}");
}

#[test]
fn failed_provision_never_leaks_bundle_bytes() {
    let sentinel = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
    let runner = FakeRunner::scripted(vec![
        output(false, &format!("Bundle[{{token={sentinel}}}]"), "denied"),
        output(true, &format!("Bundle[{{token={sentinel}}}]"), ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let first = crate::driver::provision(&adb, "s1").map(|_| ());
    assert!(first.is_err());
    let ok = crate::driver::provision(&adb, "s1").map(|t| format!("{t:?} {t}"));
    if let Ok(text) = &ok {
        assert!(!text.contains(sentinel));
    }
    for surface in [
        format!("{first:?}"),
        first
            .err()
            .map(|e| e.message().to_owned())
            .unwrap_or_default(),
    ] {
        assert!(!surface.contains(sentinel), "error leaked token: {surface}");
    }
}
