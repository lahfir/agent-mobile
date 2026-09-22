//! Shared harness for CLI integration tests: stub HTTP driver, isolated
//! HOME, and output helpers.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use agent_mobile_core::error::Failure;

static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn fail(msg: &str) -> Failure {
    Failure::local(msg.to_owned(), "fix the test")
}

pub fn tmp_home(tag: &str) -> Result<PathBuf, Failure> {
    let dir = std::env::temp_dir().join(format!(
        "am-cli-test-{}-{tag}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn request_complete(buf: &[u8]) -> bool {
    let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    buf.len() >= pos + 4 + len
}

/// A stub driver: serves one canned response per connection, then reports
/// every captured request when joined.
pub struct Stub {
    pub url: String,
    join: JoinHandle<Vec<String>>,
}

impl Stub {
    /// Every request the stub captured, in order.
    ///
    /// # Errors
    /// Returns a failure when the stub thread panicked.
    pub fn captured(self) -> Result<Vec<String>, Failure> {
        self.join.join().map_err(|_| fail("stub join"))
    }
}

pub fn stub(bodies: &[&str]) -> Result<Stub, Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let payloads: Vec<String> = bodies.iter().map(|b| (*b).to_owned()).collect();
    let join = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for payload in payloads {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            while !request_complete(&buf) {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            captured.push(String::from_utf8_lossy(&buf).into_owned());
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
        captured
    });
    Ok(Stub {
        url: format!("http://127.0.0.1:{port}"),
        join,
    })
}

pub fn run(args: &[&str], home: &PathBuf, envs: &[(&str, &str)]) -> Result<Output, Failure> {
    Command::new(env!("CARGO_BIN_EXE_agent-mobile"))
        .args(args)
        .env("HOME", home)
        .env_remove("AGENT_MOBILE_URL")
        .env_remove("AGENT_MOBILE_TOKEN")
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .output()
        .map_err(Failure::from)
}

pub fn run_wired(args: &[&str], home: &PathBuf, s: &Stub) -> Result<Output, Failure> {
    run(
        args,
        home,
        &[
            ("AGENT_MOBILE_URL", s.url.as_str()),
            ("AGENT_MOBILE_TOKEN", "tok"),
        ],
    )
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

pub fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

pub fn body_of(req: &str) -> Result<serde_json::Value, Failure> {
    let body = req
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| fail("request has no body"))?;
    serde_json::from_str(body).map_err(|e| fail(&format!("bad json body: {e}")))
}

pub const STATUS: &str = r#"{"version":"1","ok":true,"command":"status","elapsed_ms":3,"data":{"app":"com.apple.springboard","device":"Sim","os":"26.0","snapshot_id":"sid1"}}"#;

pub const SNAPSHOT: &str = r#"{"version":"1","ok":true,"command":"snapshot","elapsed_ms":7,"data":{"app":"com.x","snapshot_id":"snap1","ref_count":2,"complete":true,"settled":true,"reads":2,"text":"t","tree":{"role":"application","name":"App","value":"","ref_id":"@snap1:e1","states":[],"available_actions":[],"bounds":{"x":0.0,"y":0.0,"width":430.0,"height":930.0},"children":[{"role":"group","name":"Outer","value":"","ref_id":"@snap1:e2","states":[],"available_actions":["Tap"],"bounds":{"x":0.0,"y":0.0,"width":100.0,"height":50.0},"children":[{"role":"button","name":"Deep","value":"","ref_id":"@snap1:e3","states":[],"available_actions":["Tap"],"bounds":{"x":1.0,"y":2.0,"width":10.0,"height":10.0},"children":[]}]}]}}}"#;

#[allow(
    dead_code,
    reason = "shared fixture; only some test binaries replay it"
)]
pub const STALE: &str = r#"{"version":"1","ok":false,"command":"tap","elapsed_ms":4,"error":{"code":"STALE_REF","message":"ref gone"}}"#;
