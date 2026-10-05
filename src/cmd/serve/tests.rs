//! `finish_cleanup` ordering/retention proofs over a real `StateStore`
//! and an injected stop seam — no device, no sleeps.

use std::sync::atomic::{AtomicUsize, Ordering};

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

use super::{clear_session, finish_cleanup, register_session};
use crate::cmd::tests::TestDir;

fn store_at(tag: &str) -> (StateStore, TestDir) {
    let tmp = TestDir::new(tag);
    (StateStore::at(&tmp.0), tmp)
}

fn android_entry() -> SessionEntry {
    let mut e = SessionEntry::new(
        "http://127.0.0.1:60001".to_owned(),
        std::process::id(),
        "tok-file".to_owned(),
    );
    e.platform = Some("android".to_owned());
    e.serial = Some("emulator-5554".to_owned());
    e.forward_port = Some(50001);
    e
}

#[test]
fn original_error_plus_successful_stop_clears_everything() -> Result<(), Failure> {
    let (store, _d) = store_at("ok-clear");
    store.write_token("tok-file", "tok")?;
    store.upsert("android:x", &android_entry())?;
    let calls = AtomicUsize::new(0);
    let out = finish_cleanup(
        &store,
        "android:x",
        "tok-file",
        &android_entry(),
        Err(Failure::local("registration blew up", "retry")),
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    let err = out
        .err()
        .ok_or_else(|| Failure::local("expected err", "f"))?;
    assert!(err.message().contains("registration blew up"), "{err:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(store.entry("android:x").is_none());
    assert!(!store.token_path("tok-file").exists());
    Ok(())
}

#[test]
fn stop_failure_retains_metadata_and_token_reference() -> Result<(), Failure> {
    let (store, _d) = store_at("retain");
    store.write_token("tok-file", "tok")?;
    let entry = android_entry();
    let out = finish_cleanup(
        &store,
        "android:x",
        "tok-file",
        &entry,
        Err(Failure::local("probe died", "retry")),
        || {
            Err(Failure::local(
                "forward --remove refused",
                "free it by hand",
            ))
        },
    );
    let err = out
        .err()
        .ok_or_else(|| Failure::local("expected err", "f"))?;
    assert!(
        err.message().contains("forward --remove refused"),
        "cleanup failure must take precedence: {err:?}"
    );
    let row = store
        .entry("android:x")
        .ok_or_else(|| Failure::local("row must be retained", "f"))?;
    assert_eq!(row.serial.as_deref(), Some("emulator-5554"));
    assert_eq!(row.forward_port, Some(50001));
    assert_eq!(row.token_file, "tok-file");
    assert!(store.token_path("tok-file").exists(), "token must remain");
    Ok(())
}

#[test]
fn nonzero_app_code_stops_then_returns_code() -> Result<(), Failure> {
    let (store, _d) = store_at("code");
    let out = finish_cleanup(
        &store,
        "android:x",
        "tok-file",
        &android_entry(),
        Ok(3),
        || Ok(()),
    )?;
    assert_eq!(out, 3);
    Ok(())
}

#[test]
fn stop_failure_keeps_row_even_on_success_code() {
    let (store, _d) = store_at("code-keep");
    let out = finish_cleanup(
        &store,
        "android:x",
        "tok-file",
        &android_entry(),
        Ok(0),
        || Err(Failure::local("remove failed", "retry later")),
    );
    let err = out.err();
    assert!(err.is_some(), "cleanup failure must surface over success");
    assert!(store.entry("android:x").is_some());
}

#[test]
fn clear_session_removes_row_and_token() -> Result<(), Failure> {
    let (store, _d) = store_at("clear");
    store.write_token("tok-file", "tok")?;
    store.upsert("android:x", &android_entry())?;
    clear_session(&store, "android:x", "tok-file")?;
    assert!(store.entry("android:x").is_none());
    assert!(!store.token_path("tok-file").exists());
    Ok(())
}

#[test]
fn token_removal_failure_keeps_the_state_row() -> Result<(), Failure> {
    let (store, _d) = store_at("token-fail");
    store.write_token("tok-file", "tok")?;
    store.upsert("android:x", &android_entry())?;
    let token_path = store.token_path("tok-file");
    std::fs::remove_file(&token_path)?;
    std::fs::create_dir(&token_path)?;
    let out = finish_cleanup(
        &store,
        "android:x",
        "tok-file",
        &android_entry(),
        Err(Failure::local("launch blew up", "retry")),
        || Ok(()),
    );
    let err = out
        .err()
        .ok_or_else(|| Failure::local("expected err", "f"))?;
    assert!(
        !err.message().contains("launch blew up"),
        "cleanup failure must take precedence: {err:?}"
    );
    let _ = std::fs::remove_dir(&token_path);
    assert!(store.entry("android:x").is_some(), "state row must remain");
    Ok(())
}

fn ios_device() -> crate::platform::PlatformDevice {
    crate::platform::PlatformDevice::from_ios(agent_mobile_core::ios::Device {
        name: "iPhone 17".to_owned(),
        udid: "UDID-TEST".to_owned(),
        kind: "simulator",
        os: Some("26.0".to_owned()),
        state: Some("Booted".to_owned()),
    })
}

fn android_device(serial: &str) -> crate::platform::PlatformDevice {
    crate::platform::PlatformDevice::from_android(agent_mobile_android::AndroidTarget {
        id: "avd:test".to_owned(),
        name: "test".to_owned(),
        serial: Some(serial.to_owned()),
        kind: agent_mobile_android::AndroidDeviceKind::Emulator,
        state: agent_mobile_android::AndroidDeviceState::Other("device".to_owned()),
        model: None,
        product: None,
        os: None,
        avd: None,
    })
}

#[test]
fn selected_stale_row_survives_token_removal_failure() -> Result<(), Failure> {
    let (store, _d) = store_at("sel-token-fail");
    let mut entry = SessionEntry::new("http://127.0.0.1:1".to_owned(), 1, "tok-file".to_owned());
    entry.platform = Some("ios".to_owned());
    entry.process_started_at = Some("ps:not-this-process".to_owned());
    let key = ios_device().key();
    store.upsert(&key, &entry)?;
    let token_path = store.token_path("tok-file");
    std::fs::create_dir_all(
        token_path
            .parent()
            .ok_or_else(|| Failure::local("no parent", "f"))?,
    )?;
    std::fs::create_dir(&token_path)?;
    let err = super::reclaim_or_conflict(&store, &ios_device())
        .err()
        .map(|e| e.message().to_owned())
        .unwrap_or_default();
    assert!(!err.is_empty(), "token failure must surface");
    assert!(
        store.entry(&key).is_some(),
        "the state row must be retained"
    );
    let _ = std::fs::remove_dir(&token_path);
    Ok(())
}

#[test]
fn unrelated_cleanup_failure_keeps_row_and_token() -> Result<(), Failure> {
    let (store, _d) = store_at("unrel-keep");
    let mut entry = SessionEntry::new("http://127.0.0.1:2".to_owned(), 1, "tok-file".to_owned());
    entry.platform = Some("android".to_owned());
    entry.process_started_at = Some("ps:not-this-process".to_owned());
    store.upsert("android:other", &entry)?;
    store.write_token("tok-file", "tok")?;
    super::reclaim_or_conflict(&store, &android_device("emulator-5554"))?;
    let row = store
        .entry("android:other")
        .ok_or_else(|| Failure::local("row erased by failed cleanup", "f"))?;
    assert_eq!(row.token_file, "tok-file");
    assert!(store.token_path("tok-file").exists());
    Ok(())
}

#[test]
fn registration_fails_before_any_write_without_process_marker() {
    let (store, _d) = store_at("no-marker");
    let mut e = android_entry();
    e.process_started_at = None;
    let err = register_session(&store, "android:x", "tok-file", &e, "tok")
        .err()
        .map(|f| f.render())
        .unwrap_or_default();
    assert!(err.contains("cannot identify the serve process"), "{err}");
    assert!(
        !store.token_path("tok-file").exists(),
        "no token file may be written"
    );
    let state = store.load();
    assert!(state.devices.is_empty(), "no state row may be written");
    assert_eq!(state.default_device, None, "default stays untouched");
    assert_eq!(state.default_device_key, None);
}
