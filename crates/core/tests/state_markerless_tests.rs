//! Markerless legacy state rows: they load, but a live PID without a
//! recorded birth marker can never authorize a bearer read/send.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::StateStore;

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

fn fail(msg: &str) -> Failure {
    Failure::local(msg, "rerun the command")
}

#[test]
fn legacy_v1_row_loads_but_live_markerless_pid_fails_closed() -> Result<(), Failure> {
    let tmp = TempDir::new("legacy-v1");
    let home = tmp.0.clone();
    let store = StateStore::at(&home);
    let json = format!(
        "{{\"version\":1,\"default_device\":\"iPhone 15\",\"devices\":{{\"iPhone 15\":{{\"url\":\"http://127.0.0.1:8770\",\"pid\":{},\"token_file\":\"dev\",\"runner_pid\":null}}}}}}",
        std::process::id()
    );
    fs::create_dir_all(&home).map_err(Failure::from)?;
    fs::write(store.state_file(), json).map_err(Failure::from)?;
    assert!(!store.token_path("dev").exists());
    let state = store.load();
    let entry = state
        .devices
        .get("iPhone 15")
        .ok_or_else(|| fail("legacy row missing"))?;
    assert_eq!(entry.platform, None);
    assert_eq!(entry.serial, None);
    assert_eq!(entry.forward_port, None);
    let err = store
        .resolve_with(None, None, None)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("pre-upgrade driver session"), "{err}");
    assert!(err.contains("stop that existing"), "{err}");
    Ok(())
}

#[test]
fn dead_markerless_row_resolves_as_absent() -> Result<(), Failure> {
    let tmp = TempDir::new("legacy-dead");
    let home = tmp.0.clone();
    let store = StateStore::at(&home);
    let json = "{\"version\":1,\"default_device\":\"iPhone 15\",\"devices\":{\"iPhone 15\":{\"url\":\"http://127.0.0.1:8770\",\"pid\":4000000,\"token_file\":\"dev\",\"runner_pid\":null}}}";
    fs::create_dir_all(&home).map_err(Failure::from)?;
    fs::write(store.state_file(), json).map_err(Failure::from)?;
    let resolved = store.resolve_with(None, None, None)?;
    assert!(
        resolved.is_none(),
        "dead markerless row resolves as absent: {resolved:?}"
    );
    Ok(())
}
