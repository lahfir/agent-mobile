//! Backward-compatible default-device selection: `default_device_key`
//! migration, live-alias resolution order, and env override retention.

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

struct SelDir(std::path::PathBuf);

impl Drop for SelDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store_at(tag: &str) -> (SelDir, StateStore) {
    let dir = std::env::temp_dir().join(format!("am-sel-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_default();
    (SelDir(dir.clone()), StateStore::at(&dir))
}

fn live_entry(device_id: &str, name: &str) -> SessionEntry {
    let mut e = SessionEntry::new(
        format!(
            "http://127.0.0.1:{0}",
            40_000 + u16::try_from(device_id.len()).unwrap_or(0)
        ),
        std::process::id(),
        "t0k".to_owned(),
    );
    e.platform = Some("ios".to_owned());
    e.device_id = Some(device_id.to_owned());
    e.device_name = Some(name.to_owned());
    e
}

fn dead_entry(device_id: &str, name: &str) -> SessionEntry {
    let mut e = live_entry(device_id, name);
    e.pid = u32::MAX - 1;
    e.process_started_at = Some("ps:long-gone".to_owned());
    e
}

#[test]
fn v1_state_without_default_device_key_still_loads() -> Result<(), Failure> {
    let tmp = store_at("v1");
    let store = tmp.1.clone();
    std::fs::write(
        store.state_file(),
        r#"{"version":1,"default_device":"iPhone 17","devices":{},"pending_forwards":[]}"#,
    )?;
    let state = store.load();
    assert_eq!(state.default_device.as_deref(), Some("iPhone 17"));
    assert_eq!(state.default_device_key, None);
    Ok(())
}

#[test]
fn remember_device_selection_writes_key_and_display() -> Result<(), Failure> {
    let tmp = store_at("pair");
    let store = tmp.1.clone();
    store.remember_device_selection("ios:UDID-1", "iPhone 17")?;
    let state = store.load();
    assert_eq!(state.default_device_key.as_deref(), Some("ios:UDID-1"));
    assert_eq!(state.default_device.as_deref(), Some("iPhone 17"));
    store.remember_device("ios:UDID-9")?;
    let state = store.load();
    assert_eq!(state.default_device_key.as_deref(), Some("ios:UDID-9"));
    assert_eq!(state.default_device.as_deref(), Some("ios:UDID-9"));
    Ok(())
}

#[test]
fn live_alias_resolves_key_id_serial_and_name() -> Result<(), Failure> {
    let tmp = store_at("alias");
    let store = tmp.1.clone();
    let mut e = live_entry("UDID-1", "iPhone 17");
    e.serial = Some("emulator-5554".to_owned());
    store.upsert("ios:UDID-1", &e)?;
    for q in ["ios:UDID-1", "UDID-1", "emulator-5554", "iPhone 17"] {
        assert_eq!(
            store.resolve_live_device_key(q)?.as_deref(),
            Some("ios:UDID-1"),
            "selector {q}"
        );
    }
    assert_eq!(store.resolve_live_device_key("nope")?, None);
    Ok(())
}

#[test]
fn ambiguous_live_alias_is_usage_error() -> Result<(), Failure> {
    let tmp = store_at("ambig");
    let store = tmp.1.clone();
    let mut a = live_entry("UDID-1", "Twin");
    let mut b = live_entry("UDID-2", "Twin");
    a.serial = None;
    b.serial = None;
    store.upsert("ios:UDID-1", &a)?;
    store.upsert("ios:UDID-2", &b)?;
    assert!(store.resolve_live_device_key("Twin").is_err());
    Ok(())
}

#[test]
fn dead_rows_never_match_aliases() -> Result<(), Failure> {
    let tmp = store_at("dead");
    let store = tmp.1.clone();
    let mut e = dead_entry("UDID-1", "iPhone 17");
    e.serial = Some("emulator-5554".to_owned());
    store.upsert("ios:UDID-1", &e)?;
    assert_eq!(store.resolve_live_device_key("ios:UDID-1")?, None);
    assert_eq!(store.resolve_live_device_key("UDID-1")?, None);
    assert_eq!(store.resolve_live_device_key("iPhone 17")?, None);
    Ok(())
}

#[test]
fn legacy_display_default_resolves_canonical_live_row() -> Result<(), Failure> {
    let tmp = store_at("legacy");
    let store = tmp.1.clone();
    store.write_token("t0k", "tok")?;
    let mut state = store.load();
    state.default_device = Some("iPhone 17".to_owned());
    state
        .devices
        .insert("ios:UDID-1".to_owned(), live_entry("UDID-1", "iPhone 17"));
    std::fs::write(
        store.state_file(),
        serde_json::to_string(&state).unwrap_or_default(),
    )?;
    let ep = store
        .resolve_with(None, None, None)?
        .ok_or_else(|| Failure::local("no endpoint", "fail"))?;
    assert_eq!(ep.url, "http://127.0.0.1:40006");
    Ok(())
}

#[test]
fn env_pair_short_circuits_state_alias_resolution() -> Result<(), Failure> {
    let tmp = store_at("env");
    let store = tmp.1.clone();
    store.remember_device_selection("ios:X", "Twin")?;
    let ep = store.resolve_with(
        None,
        Some("http://127.0.0.1:9".to_owned()),
        Some("tok".to_owned()),
    )?;
    assert!(ep.is_some());
    Ok(())
}

#[test]
fn stale_default_key_falls_through_to_legacy_display_alias() -> Result<(), Failure> {
    let tmp = store_at("stalekey");
    let store = tmp.1.clone();
    store.write_token("t0k", "tok")?;
    store.upsert("ios:UDID-1", &live_entry("UDID-1", "iPhone 17"))?;
    store.remember_device_selection("ios:gone", "iPhone 17")?;
    let ep = store
        .resolve_with(None, None, None)?
        .ok_or_else(|| Failure::local("no endpoint", "fail"))?;
    assert_eq!(ep.url, "http://127.0.0.1:40006");
    Ok(())
}

#[test]
fn live_row_with_missing_token_is_an_explicit_failure() -> Result<(), Failure> {
    let tmp = store_at("notok");
    let store = tmp.1.clone();
    store.upsert("ios:UDID-1", &live_entry("UDID-1", "iPhone 17"))?;
    store.remember_device_selection("ios:UDID-1", "iPhone 17")?;
    let err = store
        .resolve_with(None, None, None)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(!err.is_empty(), "missing token must fail explicitly");
    assert!(
        !err.contains("tok"),
        "no token content in the message: {err}"
    );
    Ok(())
}
