//! Pure selection/default and entry-metadata proofs — no toolchains run
//! here; fixtures build the normalized devices directly.

use agent_mobile_android::{AndroidDeviceKind, AndroidDeviceState, AndroidScan, AndroidTarget};
use agent_mobile_core::error::Failure;
use agent_mobile_core::ios::{self, DeviceScan};
use agent_mobile_core::state::SessionEntry;

use super::{
    Platform, PlatformDevice, PlatformScan,
    android_ops::{AndroidMeta, android_entry_fields},
    default_device, select,
};

fn ios_dev(name: &str, udid: &str, kind: &'static str) -> PlatformDevice {
    PlatformDevice::from_ios(ios::Device {
        name: name.to_owned(),
        udid: udid.to_owned(),
        kind,
        os: Some("26.0".to_owned()),
        state: Some("Booted".to_owned()),
    })
}

fn and_dev(id: &str, serial: Option<&str>, state: &str) -> PlatformDevice {
    PlatformDevice::from_android(AndroidTarget {
        id: id.to_owned(),
        name: id.trim_start_matches("avd:").to_owned(),
        serial: serial.map(str::to_owned),
        kind: AndroidDeviceKind::Emulator,
        state: AndroidDeviceState::Other(state.to_owned()),
        model: None,
        product: None,
        os: None,
        avd: None,
    })
}

fn scan(devices: Vec<PlatformDevice>) -> PlatformScan {
    PlatformScan {
        devices,
        notes: vec![],
    }
}

#[test]
fn key_beats_raw_id_and_name() -> Result<(), Failure> {
    let scan = scan(vec![
        ios_dev("iPhone 17", "UDID-1", "simulator"),
        and_dev("emulator-5554", Some("emulator-5554"), "device"),
    ]);
    assert_eq!(select(&scan, "ios:UDID-1")?.id(), "UDID-1");
    assert_eq!(select(&scan, "UDID-1")?.platform(), Platform::Ios);
    assert_eq!(select(&scan, "iPhone 17")?.id(), "UDID-1");
    assert_eq!(
        select(&scan, "android:emulator-5554")?.id(),
        "emulator-5554"
    );
    Ok(())
}

#[test]
fn stable_id_beats_anothers_display_name() -> Result<(), Failure> {
    let scan = scan(vec![
        ios_dev("emulator-5554", "UDID-9", "simulator"),
        and_dev("emulator-5554", Some("emulator-5554"), "device"),
    ]);
    let picked = select(&scan, "emulator-5554")?;
    assert_eq!(picked.platform(), Platform::Android);
    assert_eq!(picked.id(), "emulator-5554");
    Ok(())
}

#[test]
fn duplicate_display_name_lists_every_key() {
    let scan = scan(vec![
        ios_dev("iPhone 17", "UDID-1", "simulator"),
        ios_dev("iPhone 17", "UDID-2", "simulator"),
    ]);
    let err = select(&scan, "iPhone 17").err();
    let text = format!("{err:?}");
    assert!(text.contains("ios:UDID-1"), "{text}");
    assert!(text.contains("ios:UDID-2"), "{text}");
}

#[test]
fn duplicate_raw_id_forces_key() {
    let scan = scan(vec![
        and_dev("dup-serial", Some("dup-serial"), "device"),
        PlatformDevice::from_android(AndroidTarget {
            id: "dup-serial".to_owned(),
            name: "other".to_owned(),
            serial: Some("dup-serial".to_owned()),
            kind: AndroidDeviceKind::Usb,
            state: AndroidDeviceState::Device,
            model: None,
            product: None,
            os: None,
            avd: None,
        }),
    ]);
    let err = select(&scan, "dup-serial").err();
    let text = format!("{err:?}");
    assert!(text.contains("android:dup-serial"), "{text}");
    assert!(select(&scan, "android:dup-serial").is_ok());
}

#[test]
fn unknown_query_is_usage() {
    let scan = scan(vec![ios_dev("iPhone 17", "UDID-1", "simulator")]);
    let err = select(&scan, "nothing").err();
    assert!(format!("{err:?}").contains("devices"));
}

#[test]
fn default_prefers_booted_iphone_sim_then_tier_order() {
    let android_only = scan(vec![and_dev("avd:x", None, "shutdown")]);
    assert_eq!(
        default_device(&android_only)
            .map(|d| d.id().to_owned())
            .as_deref(),
        Some("avd:x")
    );
    let both = scan(vec![
        and_dev("emulator-5554", Some("emulator-5554"), "device"),
        ios_dev("Pixel phone", "UDID-P", "device"),
        ios_dev("iPad", "UDID-i", "simulator"),
        ios_dev("iPhone 17", "UDID-1", "simulator"),
    ]);
    assert_eq!(
        default_device(&both).map(|d| d.id().to_owned()).as_deref(),
        Some("UDID-1")
    );
    let no_iphone = scan(vec![
        and_dev("emulator-5554", Some("emulator-5554"), "device"),
        ios_dev("iPad", "UDID-i", "simulator"),
    ]);
    assert_eq!(
        default_device(&no_iphone)
            .map(|d| d.id().to_owned())
            .as_deref(),
        Some("UDID-i")
    );
    let physical_only_ios = scan(vec![
        ios_dev("Pixel phone", "UDID-P", "device"),
        and_dev("emulator-5554", Some("emulator-5554"), "device"),
    ]);
    assert_eq!(
        default_device(&physical_only_ios)
            .map(|d| d.id().to_owned())
            .as_deref(),
        Some("UDID-P")
    );
    let ready_android = scan(vec![and_dev(
        "emulator-5554",
        Some("emulator-5554"),
        "device",
    )]);
    assert_eq!(
        default_device(&ready_android)
            .map(|d| d.id().to_owned())
            .as_deref(),
        Some("emulator-5554")
    );
    let shutdown_first = scan(vec![
        PlatformDevice::from_ios(ios::Device {
            name: "iPhone 17".to_owned(),
            udid: "UDID-SHUT".to_owned(),
            kind: "simulator",
            os: Some("26.0".to_owned()),
            state: Some("Shutdown".to_owned()),
        }),
        ios_dev("iPhone 17 Pro", "UDID-BOOT", "simulator"),
    ]);
    assert_eq!(
        default_device(&shutdown_first)
            .map(|d| d.id().to_owned())
            .as_deref(),
        Some("UDID-BOOT")
    );
}

#[test]
fn device_normalization_fields() {
    let ios = ios_dev("iPhone 17", "UDID-1", "simulator");
    assert_eq!(ios.key(), "ios:UDID-1");
    assert_eq!(ios.legacy_key(), Some("iPhone 17"));
    assert_eq!(ios.fixed_addr().as_deref(), Some("127.0.0.1:8770"));
    let and = and_dev("avd:x", None, "shutdown");
    assert_eq!(and.key(), "android:avd:x");
    assert_eq!(and.legacy_key(), None);
    assert_eq!(and.fixed_addr(), None);
    assert_eq!(and.kind(), "emulator");
    assert_eq!(and.state(), Some("shutdown"));
}

#[test]
fn android_entry_fields_carries_cleanup_metadata() {
    let mut e = SessionEntry::new(
        "http://127.0.0.1:60001".to_owned(),
        42,
        "tok-android".to_owned(),
    );
    android_entry_fields(
        &mut e,
        AndroidMeta {
            serial: Some("emulator-5554"),
            forward_port: Some(50001),
            device_port: Some(45678),
            bridge_port: Some(60001),
            apk_source: Some(std::path::Path::new("/repo/app-debug.apk")),
            emulator_pid: Some(50564),
        },
        std::path::Path::new("/tmp/avd.log"),
    );
    assert_eq!(e.serial.as_deref(), Some("emulator-5554"));
    assert_eq!(e.forward_port, Some(50001));
    assert_eq!(e.device_port, Some(45678));
    assert_eq!(crate::platform::android_ops::android_device_port(&e), 45678);
    e.device_port = None;
    assert_eq!(
        crate::platform::android_ops::android_device_port(&e),
        agent_mobile_android::LEGACY_DEVICE_PORT
    );
    assert_eq!(e.bridge_port, Some(60001));
    assert_eq!(e.apk_source.as_deref(), Some("/repo/app-debug.apk"));
    assert_eq!(e.emulator_pid, Some(50564));
    assert_eq!(e.log_file.as_deref(), Some("/tmp/avd.log"));
    let raw = serde_json::to_string(&e).unwrap_or_default();
    assert!(!raw.contains("canary"), "{raw}");
}

#[test]
fn collect_keeps_working_side_and_notes_the_failed_one() {
    let ios = DeviceScan {
        devices: vec![],
        notes: vec!["devicectl slow".to_owned()],
    };
    let android = AndroidScan {
        targets: vec![AndroidTarget {
            id: "avd:x".to_owned(),
            name: "x".to_owned(),
            serial: None,
            kind: AndroidDeviceKind::Emulator,
            state: AndroidDeviceState::Other("shutdown".to_owned()),
            model: None,
            product: None,
            os: None,
            avd: Some("x".to_owned()),
        }],
        notes: vec![],
    };
    let (devices, notes) = super::discovery::collect(Ok(ios), Ok(android));
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id(), "avd:x");
    assert!(notes.iter().any(|n| n == "ios: devicectl slow"));
    let (devices2, notes2) = super::discovery::collect(
        Err(Failure::local("no xcode", "install Xcode")),
        Ok(AndroidScan {
            targets: vec![],
            notes: vec![],
        }),
    );
    assert!(devices2.is_empty());
    assert!(notes2.iter().any(|n| n.starts_with("ios:")), "{notes2:?}");
}

#[test]
fn devices_serving_prefers_key_then_legacy_name() {
    use agent_mobile_core::state::{SessionEntry, State};
    use std::collections::BTreeMap;

    let live = SessionEntry::new(
        "http://127.0.0.1:8770".to_owned(),
        std::process::id(),
        "t".to_owned(),
    );
    let mut devices = BTreeMap::new();
    devices.insert("ios:UDID-1".to_owned(), live.clone());
    devices.insert("iPhone 17".to_owned(), {
        let mut stale = SessionEntry::new("http://127.0.0.1:9000".to_owned(), 1, "t".to_owned());
        stale.pid = 1;
        stale
    });
    let state = State {
        version: 1,
        default_device: None,
        default_device_key: None,
        devices,
        pending_forwards: Vec::new(),
    };
    let d = ios_dev("iPhone 17", "UDID-1", "simulator");
    assert_eq!(
        crate::cmd::devices::serving_url(&state, &d),
        Some("http://127.0.0.1:8770")
    );
    let mut devices2 = BTreeMap::new();
    devices2.insert("iPhone 17".to_owned(), live);
    let state2 = State {
        version: 1,
        default_device: None,
        default_device_key: None,
        devices: devices2,
        pending_forwards: Vec::new(),
    };
    assert_eq!(
        crate::cmd::devices::serving_url(&state2, &d),
        Some("http://127.0.0.1:8770")
    );
    let and = and_dev("avd:x", None, "shutdown");
    assert_eq!(crate::cmd::devices::serving_url(&state2, &and), None);
}

mod extra;
