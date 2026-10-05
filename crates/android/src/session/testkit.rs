use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use agent_mobile_core::error::Failure;

use super::AndroidAdapter;
use crate::adb::{Adb, CommandOutput, CommandRunner};
use crate::testkit::{FakeRunner, output};

pub(super) const TOKEN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
pub(super) const SVC: &str = "a.b/.C:com.lahfir.agentmobile.driver/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService";

pub(super) fn apk_fixture() -> Result<PathBuf, Failure> {
    let dir = std::env::temp_dir().join(format!("am-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(Failure::from)?;
    let apk = dir.join("driver.apk");
    std::fs::write(&apk, b"apk").map_err(Failure::from)?;
    Ok(apk)
}

pub(super) fn upstream_body_at(port: u16, body: &str) -> u16 {
    let body = body.to_owned();
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
        return 0;
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    thread::spawn(move || {
        while let Ok((mut sock, _)) = listener.accept() {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 8192];
            let _ = sock.read(&mut buf);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes());
        }
    });
    port
}

pub(super) fn adapter_with() -> Result<(AndroidAdapter, std::sync::Arc<FakeRunner>), Failure> {
    adapter_with_extra(vec![
        output(true, "other tcp:1 tcp:2\ns1 tcp:{LOCAL} tcp:9876", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
    ])
}

/// Runner that binds a one-shot status upstream on the exact port the
/// `forward --no-rebind tcp:P` call carries — the reserved local port —
/// so the real probe connects to our fake.
pub(super) struct BindOnForwardRunner {
    inner: std::sync::Arc<FakeRunner>,
    body: &'static str,
}

impl CommandRunner for BindOnForwardRunner {
    fn run(
        &self,
        program: &std::path::Path,
        args: &[&str],
        timeout: std::time::Duration,
    ) -> Result<CommandOutput, Failure> {
        if let Some(i) = args.iter().position(|a| *a == "--no-rebind")
            && let Some(p) = args.get(i + 1).and_then(|a| a.strip_prefix("tcp:"))
            && let Ok(p) = p.parse::<u16>()
        {
            upstream_body_at(p, self.body);
        }
        self.inner.run(program, args, timeout)
    }
}

pub(super) const STATUS_BODY: &str = "{\"version\":\"1\",\"ok\":true,\"command\":\"status\",\"elapsed_ms\":1,\"data\":{\"app\":\"com.x\",\"snapshot_id\":\"\",\"device\":\"d\",\"os\":\"1\"}}";

pub(super) fn adapter_with_extra(
    extra: Vec<CommandOutput>,
) -> Result<(AndroidAdapter, std::sync::Arc<FakeRunner>), Failure> {
    adapter_with_body(extra, STATUS_BODY)
}

pub(super) fn adapter_with_body(
    extra: Vec<CommandOutput>,
    body: &'static str,
) -> Result<(AndroidAdapter, std::sync::Arc<FakeRunner>), Failure> {
    let mut replies = vec![
        output(true, "device\n", ""),
        output(true, "Success\n", ""),
        output(true, SVC, ""),
        output(true, "1", ""),
        output(true, SVC, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
        output(
            true,
            &format!("result=Bundle[{{token={TOKEN} port=9876}}]"),
            "",
        ),
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2\ns1 tcp:{LOCAL} tcp:9876", ""),
    ];
    replies.extend(extra);
    let inner = FakeRunner::scripted(replies);
    let runner = std::sync::Arc::new(BindOnForwardRunner {
        inner: inner.clone(),
        body,
    });
    let apk = apk_fixture()?;
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner),
        PathBuf::from("emulator"),
        Some(apk),
    );
    Ok((adapter, inner))
}

pub(super) fn reserved_port(calls: &[Vec<String>]) -> u16 {
    calls
        .iter()
        .find_map(|c| {
            let i = c.iter().position(|a| a == "--no-rebind")?;
            c.get(i + 1)?.strip_prefix("tcp:")?.parse().ok()
        })
        .unwrap_or(0)
}
