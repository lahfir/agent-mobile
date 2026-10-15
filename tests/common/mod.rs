//! Shared harness for CLI integration tests: stub HTTP driver, isolated
//! HOME, and output helpers.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
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

/// Spawn a CLI with all three standard streams piped for protocol tests.
#[allow(dead_code, reason = "only protocol suites need piped stdio")]
pub fn spawn_stdio(
    args: &[&str],
    home: &PathBuf,
    envs: &[(&str, &str)],
) -> Result<std::process::Child, Failure> {
    Command::new(env!("CARGO_BIN_EXE_agent-mobile"))
        .args(args)
        .env("HOME", home)
        .env_remove("AGENT_MOBILE_URL")
        .env_remove("AGENT_MOBILE_TOKEN")
        .env_remove("AGENT_MOBILE_MCP_CALL_SECS")
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Failure::from)
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

/// A channel-controlled HTTP driver: no clock sleeps and no early second reply.
#[allow(dead_code, reason = "only MCP recovery tests hold driver replies")]
pub fn held_driver() -> Result<(String, Receiver<String>, Sender<&'static str>), Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}", listener.local_addr()?);
    let (started, observed) = mpsc::channel();
    let (release, responses) = mpsc::channel::<&'static str>();
    std::thread::spawn(move || {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let request = read_http(&mut stream);
            let Ok(request) = request else { return };
            if started.send(request).is_err() {
                return;
            }
            let Ok(body) = responses.recv() else { return };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Ok((url, observed, release))
}

fn read_http(stream: &mut std::net::TcpStream) -> std::io::Result<String> {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        request.push_str(&line);
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    request.push_str(&String::from_utf8_lossy(&body));
    Ok(request)
}
