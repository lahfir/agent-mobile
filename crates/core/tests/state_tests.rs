//! Session-store behavior: creation-time permissions, env override precedence,
//! corrupt-state staleness, and token redaction.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_mobile_core::error::{EXIT_USAGE, Failure};
use agent_mobile_core::secret::write_secret;
use agent_mobile_core::state::{SessionEntry, StateStore};

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("am-state-{tag}-{}-{n}", std::process::id()));
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entry(url: &str) -> SessionEntry {
    SessionEntry {
        url: url.to_owned(),
        pid: 4242,
        token_file: "dev".to_owned(),
        runner_pid: None,
    }
}

fn fail(msg: &str) -> Failure {
    Failure::local(msg.to_owned(), "fix the test")
}

#[test]
fn secret_file_created_0600() -> Result<(), Failure> {
    let tmp = TempDir::new("mode");
    let path = tmp.0.join("deep/nested/token");
    write_secret(&path, "canary-token")?;
    let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "secret file must be 0600 at creation");
    Ok(())
}

#[test]
fn parent_dirs_created_0700() -> Result<(), Failure> {
    let tmp = TempDir::new("dirs");
    let file = tmp.0.join("a/b/c/tok");
    write_secret(&file, "x")?;
    for dir in [tmp.0.join("a"), tmp.0.join("a/b"), tmp.0.join("a/b/c")] {
        let mode = fs::metadata(&dir)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{} must be 0700", dir.display());
    }
    Ok(())
}

#[test]
fn second_secret_write_fails_closed() -> Result<(), Failure> {
    let tmp = TempDir::new("exists");
    let path = tmp.0.join("tok");
    write_secret(&path, "first")?;
    assert!(write_secret(&path, "second").is_err());
    assert_eq!(fs::read_to_string(&path)?, "first");
    Ok(())
}

#[test]
fn env_override_wins_without_mutating_state() -> Result<(), Failure> {
    let tmp = TempDir::new("env");
    let store = StateStore::at(&tmp.0);
    store.write_token("dev", "state-token")?;
    store.upsert("sim", &entry("http://127.0.0.1:8770"))?;
    let out = store.resolve_with(
        Some("sim"),
        Some("http://env:9".to_owned()),
        Some("env-token".to_owned()),
    )?;
    let Some(ep) = out else {
        return Err(fail("expected an endpoint"));
    };
    assert_eq!(ep.url, "http://env:9");
    assert_eq!(ep.token(), "env-token");
    let raw = fs::read_to_string(store.state_file())?;
    assert!(raw.contains("http://127.0.0.1:8770"));
    assert!(!raw.contains("env:9"));
    assert!(
        !raw.contains("state-token"),
        "state file must not hold tokens"
    );
    Ok(())
}

#[test]
fn partial_env_override_completes_from_state() -> Result<(), Failure> {
    let tmp = TempDir::new("partial");
    let store = StateStore::at(&tmp.0);
    store.write_token("dev", "state-token")?;
    store.upsert("sim", &entry("http://127.0.0.1:8770"))?;
    let out = store.resolve_with(Some("sim"), Some("http://env:9".to_owned()), None)?;
    let Some(ep) = out else {
        return Err(fail("expected an endpoint"));
    };
    assert_eq!(ep.url, "http://env:9");
    assert_eq!(ep.token(), "state-token");
    Ok(())
}

#[test]
fn corrupt_state_loads_stale() -> Result<(), Failure> {
    let tmp = TempDir::new("corrupt");
    fs::create_dir_all(&tmp.0)?;
    fs::write(tmp.0.join("state.json"), b"{not json")?;
    let store = StateStore::at(&tmp.0);
    assert!(store.load().devices.is_empty());
    match store.resolve_with(Some("sim"), None, None)? {
        None => {}
        Some(_) => return Err(fail("corrupt state must not resolve")),
    }
    Ok(())
}

#[test]
fn canary_token_never_appears_in_debug() -> Result<(), Failure> {
    let tmp = TempDir::new("redact");
    let store = StateStore::at(&tmp.0);
    store.write_token("dev", "canary-7f3c9a-token")?;
    store.upsert("sim", &entry("http://127.0.0.1:8770"))?;
    let out = store.resolve_with(Some("sim"), None, None)?;
    let Some(ep) = out else {
        return Err(fail("expected an endpoint"));
    };
    let dbg = format!("{ep:?}");
    assert!(!dbg.contains("canary-7f3c9a-token"), "Debug leaked a token");
    assert!(dbg.contains("127.0.0.1:8770"));
    Ok(())
}

#[test]
fn env_token_without_url_is_usage_error() -> Result<(), Failure> {
    let tmp = TempDir::new("tokonly");
    let store = StateStore::at(&tmp.0);
    match store.resolve_with(Some("sim"), None, Some("tok".to_owned())) {
        Err(f) => assert_eq!(f.exit_code(), EXIT_USAGE),
        Ok(_) => return Err(fail("token without url must be a usage error")),
    }
    Ok(())
}

#[test]
fn env_url_without_token_or_state_is_usage_error() -> Result<(), Failure> {
    let tmp = TempDir::new("urlonly");
    let store = StateStore::at(&tmp.0);
    match store.resolve_with(Some("sim"), Some("http://env:1".to_owned()), None) {
        Err(f) => assert_eq!(f.exit_code(), EXIT_USAGE),
        Ok(_) => return Err(fail("url without token must be a usage error")),
    }
    Ok(())
}

#[test]
fn upsert_remove_and_default_device_round_trip() -> Result<(), Failure> {
    let tmp = TempDir::new("roundtrip");
    let store = StateStore::at(&tmp.0);
    store.write_token("dev", "tok")?;
    store.upsert("sim", &entry("http://a:1"))?;
    assert!(store.entry("sim").is_some());
    store.remember_device("sim")?;
    match store.resolve_with(None, None, None)? {
        Some(ep) => assert_eq!(ep.url, "http://a:1"),
        None => return Err(fail("default device must resolve")),
    }
    store.remove("sim")?;
    assert!(store.entry("sim").is_none());
    match store.resolve_with(None, None, None)? {
        None => {}
        Some(_) => return Err(fail("removed entry must not resolve")),
    }
    Ok(())
}
