//! Session assembly order, owned cleanup, token secrecy.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use agent_mobile_core::error::Failure;

use super::AndroidAdapter;
use crate::adb::{Adb, CommandOutput};
use crate::testkit::{FakeRunner, output};

const TOKEN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";

fn apk_fixture() -> Result<PathBuf, Failure> {
    let dir = std::env::temp_dir().join(format!("am-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(Failure::from)?;
    let apk = dir.join("driver.apk");
    std::fs::write(&apk, b"apk").map_err(Failure::from)?;
    Ok(apk)
}

fn upstream_status() -> u16 {
    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
        return 0;
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    thread::spawn(move || {
        while let Ok((mut sock, _)) = listener.accept() {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 8192];
            let _ = sock.read(&mut buf);
            let body = "{\"version\":\"1\",\"ok\":true,\"command\":\"status\",\"elapsed_ms\":1,\"data\":{\"app\":\"com.x\",\"snapshot_id\":\"\",\"device\":\"d\",\"os\":\"1\"}}";
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes());
        }
    });
    port
}

fn adapter_with(port: u16) -> Result<(AndroidAdapter, std::sync::Arc<FakeRunner>), Failure> {
    adapter_with_extra(port, vec![])
}

fn adapter_with_extra(
    port: u16,
    extra: Vec<CommandOutput>,
) -> Result<(AndroidAdapter, std::sync::Arc<FakeRunner>), Failure> {
    let mut replies = vec![
        output(true, "device\n", ""),
        output(true, "Success\n", ""),
        output(true, &format!("result=Bundle[{{token={TOKEN}}}]"), ""),
        output(
            true,
            "a.b/.C:com.lahfir.agentmobile.driver/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService",
            "",
        ),
        output(true, "1", ""),
        output(
            true,
            "a.b/.C:com.lahfir.agentmobile.driver/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService",
            "",
        ),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
        output(true, "other tcp:1 tcp:2", ""),
        output(true, &port.to_string(), ""),
        output(
            true,
            &format!("other tcp:1 tcp:2\ns1 tcp:{port} tcp:8770"),
            "",
        ),
    ];
    replies.extend(extra);
    let runner = FakeRunner::scripted(replies);
    let apk = apk_fixture()?;
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        Some(apk),
    );
    Ok((adapter, runner))
}

#[test]
fn session_starts_and_closes_only_its_forward() -> Result<(), Failure> {
    let port = upstream_status();
    let (adapter, runner) = adapter_with(port)?;
    let session = adapter.start_session("s1")?;
    assert_eq!(session.token(), TOKEN);
    assert_eq!(session.serial(), "s1");
    assert_eq!(session.forward_port(), port);
    assert_ne!(session.local_port(), port);
    assert!(session.url().contains(&session.local_port().to_string()));
    let dbg = format!("{session:?}");
    assert!(!dbg.contains(TOKEN));
    let local = session.local_port();
    session.close()?;
    let calls = runner.calls();
    let remove = calls.iter().rfind(|c| c.contains(&"--remove".to_owned()));
    let remove = remove.ok_or_else(|| Failure::local("forward never removed", "fail"))?;
    assert_eq!(
        remove,
        &["-s", "s1", "forward", "--remove", &format!("tcp:{port}")]
    );
    assert!(TcpStream::connect(("127.0.0.1", local)).is_err());
    Ok(())
}

#[test]
fn session_requires_device_state() {
    let runner = FakeRunner::scripted(vec![output(true, "offline\n", "")]);
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        None,
    );
    assert!(adapter.start_session("s1").is_err());
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
    let err = adapter
        .start_session("s1")
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
    let err = adapter
        .start_session("s1")
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("unauthorized"), "{err}");
    assert!(err.contains("debugging"), "{err}");
}

#[test]
fn close_failure_leaves_drop_retry() -> Result<(), Failure> {
    let port = upstream_status();
    let (adapter, runner) = adapter_with_extra(
        port,
        vec![output(false, "", "remove failed"), output(true, "", "")],
    )?;
    let session = adapter.start_session("s1")?;
    assert!(session.close().is_err());
    let removes = runner
        .calls()
        .iter()
        .filter(|c| c.contains(&"--remove".to_owned()))
        .count();
    assert_eq!(removes, 2);
    Ok(())
}
