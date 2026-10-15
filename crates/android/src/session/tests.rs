//! Session assembly order, owned cleanup, token secrecy.

use std::net::TcpStream;
use std::path::PathBuf;

use agent_mobile_core::error::Failure;

use crate::adb::Adb;
use crate::session::AndroidAdapter;
use crate::session::testkit::*;
use crate::testkit::{FakeRunner, output};

#[test]
fn session_starts_and_closes_only_its_forward() -> Result<(), Failure> {
    let (adapter, runner) = adapter_with()?;
    let session = start_test_session(&adapter, "s1")?;
    assert_eq!(session.token(), TOKEN);
    assert_eq!(session.serial(), "s1");
    assert_ne!(session.forward_port(), 0);
    assert_eq!(session.device_port(), 9876);
    assert_ne!(session.local_port(), 0);
    assert!(session.url().contains(&session.local_port().to_string()));
    let dbg = format!("{session:?}");
    assert!(!dbg.contains(TOKEN));
    let local = session.local_port();
    let fwd = session.forward_port();
    session.close()?;
    let calls = runner.calls();
    let remove = calls.iter().rfind(|c| c.contains(&"--remove".to_owned()));
    let remove = remove.ok_or_else(|| Failure::local("forward never removed", "fail"))?;
    assert_eq!(
        remove,
        &["-s", "s1", "forward", "--remove", &format!("tcp:{fwd}")]
    );
    assert!(TcpStream::connect(("127.0.0.1", local)).is_err());
    Ok(())
}

/// The local port `create_forward` was issued with — the `tcp:` arg after
/// `--no-rebind` in the recorded calls.
#[test]
fn session_requires_device_state() {
    let runner = FakeRunner::scripted(vec![output(true, "offline\n", "")]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        None,
    );
    assert!(start_test_session(&adapter, "s1").is_err());
    assert_eq!(runner.calls().len(), 1);
}

#[test]
fn session_reports_offline_even_when_get_state_nonzero() {
    let runner = FakeRunner::scripted(vec![output(false, "", "error: device offline")]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner),
        PathBuf::from("emulator"),
        None,
    );
    let err = start_test_session(&adapter, "s1")
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("offline"), "{err}");
    assert!(err.contains("reconnect"), "{err}");
}

#[test]
fn session_reports_unauthorized_on_stderr_state() {
    let runner = FakeRunner::scripted(vec![output(
        false,
        "",
        "error: device unauthorized. This adb server's $ADB_VENDOR_KEYS is not set",
    )]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner),
        PathBuf::from("emulator"),
        None,
    );
    let err = start_test_session(&adapter, "s1")
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("unauthorized"), "{err}");
    assert!(err.contains("debugging"), "{err}");
}

#[test]
fn close_failure_leaves_drop_retry() -> Result<(), Failure> {
    let (adapter, runner) = adapter_with_extra(vec![
        output(true, "s1 tcp:{LOCAL} tcp:9876", ""),
        output(false, "", "remove failed"),
        output(true, "s1 tcp:{LOCAL} tcp:9876", ""),
        output(true, "s1 tcp:{LOCAL} tcp:9876", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
    ])?;
    let session = start_test_session(&adapter, "s1")?;
    assert!(session.close().is_err());
    let removes = runner
        .calls()
        .iter()
        .filter(|c| c.contains(&"--remove".to_owned()))
        .count();
    assert_eq!(removes, 2);
    Ok(())
}

#[test]
fn remove_owned_forward_tolerates_already_gone_row() {
    let runner = FakeRunner::scripted(vec![output(true, "s1 tcp:1 tcp:9999", "")]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        None,
    );
    assert!(adapter.remove_owned_forward("s1", 5000, 8770).is_ok());
    let calls = runner.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][2], "forward");
    assert_eq!(calls[0][3], "--list");
}

#[test]
fn remove_owned_forward_never_touches_foreign_remote() {
    let runner = FakeRunner::scripted(vec![output(true, "s1 tcp:5000 tcp:9999", "")]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        None,
    );
    assert!(adapter.remove_owned_forward("s1", 5000, 8770).is_ok());
    let calls = runner.calls();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].contains(&"--remove".to_owned()));
}

#[test]
fn remove_owned_forward_errors_when_row_persists() {
    let runner = FakeRunner::scripted(vec![
        output(true, "s1 tcp:5000 tcp:8770", ""),
        output(true, "", ""),
        output(true, "s1 tcp:5000 tcp:8770", ""),
    ]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        None,
    );
    assert!(adapter.remove_owned_forward("s1", 5000, 8770).is_err());
}

#[test]
fn session_reports_running_state() -> Result<(), Failure> {
    let (adapter, _r) = adapter_with()?;
    let session = start_test_session(&adapter, "s1")?;
    assert!(session.is_running());
    session.close()?;
    Ok(())
}
