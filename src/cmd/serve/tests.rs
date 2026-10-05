//! `finish_cleanup` ordering/retention proofs over a real `StateStore`
//! and an injected stop seam — no device, no sleeps.

use std::sync::atomic::{AtomicUsize, Ordering};

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

use super::{clear_session, finish_cleanup};

fn store_at(tag: &str) -> (StateStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("am-serve-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (StateStore::at(&dir), dir)
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
    clear_session(&store, "android:x", "tok-file");
    assert!(store.entry("android:x").is_none());
    assert!(!store.token_path("tok-file").exists());
    Ok(())
}
