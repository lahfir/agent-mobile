//! Runtime-selection and cleanup diagnostics proofs (part 2 of the
//! platform tests — split to satisfy the 400-line rule).

use agent_mobile_android::{AndroidDeviceKind, AndroidDeviceState, AndroidScan, AndroidTarget};
use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::state::SessionEntry;

use super::{ios_dev, scan};
use crate::platform::{
    PlatformDevice, android_ops, cleanup_stale, discovery, resolve_from, select,
};
#[test]
fn cross_platform_duplicate_name_lists_both_keys() {
    let scan = scan(vec![
        ios_dev("Shared", "UDID-7", "simulator"),
        PlatformDevice::from_android(AndroidTarget {
            id: "emulator-9".to_owned(),
            name: "Shared".to_owned(),
            serial: Some("emulator-9".to_owned()),
            kind: AndroidDeviceKind::Emulator,
            state: AndroidDeviceState::Device,
            model: None,
            product: None,
            os: None,
            avd: None,
        }),
    ]);
    let err = select(&scan, "Shared").err();
    let text = format!("{err:?}");
    assert!(text.contains("ios:UDID-7"), "{text}");
    assert!(text.contains("android:emulator-9"), "{text}");
}

#[test]
fn avd_target_always_boots_even_when_correlated_running() {
    use std::cell::Cell;
    use std::path::PathBuf;

    let target = AndroidTarget {
        id: "avd:agent-mobile-api37".to_owned(),
        name: "agent-mobile-api37".to_owned(),
        serial: Some("emulator-5554".to_owned()),
        kind: AndroidDeviceKind::Emulator,
        state: AndroidDeviceState::Device,
        model: None,
        product: None,
        os: Some("16".to_owned()),
        avd: Some("agent-mobile-api37".to_owned()),
    };
    let booted = Cell::new(false);
    let (serial, pid) = android_ops::android_serial(&target, |avd| {
        assert_eq!(avd, "agent-mobile-api37");
        booted.set(true);
        Ok(agent_mobile_android::BootedAvd {
            serial: "emulator-5554".to_owned(),
            pid: 0,
            log: PathBuf::from("/tmp/avd.log"),
        })
    })
    .unwrap_or_else(|_| (String::new(), None));
    assert!(booted.get(), "running AVD must still prove boot_completed");
    assert_eq!(serial, "emulator-5554");
    assert_eq!(pid, None);
}

#[test]
fn non_avd_serial_gates_state_and_skips_boot() {
    let target = AndroidTarget {
        id: "0a1b2c".to_owned(),
        name: "usb phone".to_owned(),
        serial: Some("0a1b2c".to_owned()),
        kind: AndroidDeviceKind::Usb,
        state: AndroidDeviceState::Device,
        model: None,
        product: None,
        os: None,
        avd: None,
    };
    let (serial, _) = android_ops::android_serial(&target, |_| {
        Err(agent_mobile_core::error::Failure::local(
            "boot must not run",
            "test",
        ))
    })
    .unwrap_or_default();
    assert_eq!(serial, "0a1b2c");
    let mut offline = target.clone();
    offline.state = AndroidDeviceState::Offline;
    let err = android_ops::android_serial(&offline, |_| {
        Err(agent_mobile_core::error::Failure::local(
            "boot must not run",
            "test",
        ))
    });
    assert!(err.err().is_some_and(|e| e.message().contains("offline")));
}

#[test]
fn cleanup_stale_rejects_unknown_platform_and_keeps_remedy() {
    let mut e = SessionEntry::new("u".to_owned(), 1, "t".to_owned());
    e.platform = Some("fuchsia".to_owned());
    let err = cleanup_stale(&e).err();
    let text = format!("{err:?}");
    assert!(text.contains("unrecognized platform"), "{text}");
    let mut a = SessionEntry::new("u".to_owned(), 1, "t".to_owned());
    a.platform = Some("android".to_owned());
    let err2 = cleanup_stale(&a).err();
    let text2 = format!("{err2:?}");
    assert!(text2.contains("forward --remove tcp:<port>"), "{text2}");
}

#[test]
fn resolve_from_surfaces_platform_note_for_prefixed_miss() {
    let mut s = scan(vec![]);
    s.notes = vec!["android: adb not found".to_owned()];
    let err = resolve_from(&s, "android:emulator-5554").err();
    let text = format!("{err:?}");
    assert!(text.contains("adb not found"), "{text}");
    assert!(text.contains("setup-android-sdk.sh"), "{text}");
    let err2 = resolve_from(&s, "ios:UDID").err();
    let text2 = format!("{err2:?}");
    assert!(
        text2.contains("unknown device"),
        "no ios note → plain miss: {text2}"
    );
    let mut s2 = scan(vec![]);
    s2.notes = vec!["ios: simctl missing".to_owned()];
    let err3 = resolve_from(&s2, "ios:UDID-1").err();
    assert!(format!("{err3:?}").contains("Xcode"));
    let err4 = resolve_from(&s, "avd:x").err();
    assert!(format!("{err4:?}").contains("setup-android-sdk.sh"));
    let unknown = resolve_from(&s, "plain-miss").err();
    assert!(format!("{unknown:?}").contains("unknown device"));
}

#[test]
fn combine_reports_each_side_independently() {
    let ios_ok = Ok(ios::DeviceScan {
        devices: vec![],
        notes: vec![],
    });
    let and_ok = Ok(AndroidScan {
        targets: vec![],
        notes: vec![],
    });
    let combined = discovery::combine(ios_ok, and_ok);
    assert!(combined.is_ok());
    let one = discovery::combine(
        Err(Failure::local("no xcode", "install Xcode")),
        Ok(AndroidScan {
            targets: vec![],
            notes: vec![],
        }),
    );
    let scan = one.ok();
    assert!(scan.is_some_and(|s| s.notes.iter().any(|n| n.starts_with("ios:"))));
    let two = discovery::combine(
        Ok(ios::DeviceScan {
            devices: vec![],
            notes: vec![],
        }),
        Err(Failure::local("no adb", "setup")),
    );
    assert!(
        two.ok()
            .is_some_and(|s| s.notes.iter().any(|n| n.starts_with("android:")))
    );
    let both = discovery::combine(
        Err(Failure::local("no xcode", "install Xcode")),
        Err(Failure::local("no adb", "setup")),
    );
    let text = format!("{:?}", both.err());
    assert!(text.contains("no xcode"), "{text}");
    assert!(text.contains("no adb"), "{text}");
    assert!(text.contains("Xcode"), "{text}");
    assert!(text.contains("setup-android-sdk.sh"), "{text}");
}
