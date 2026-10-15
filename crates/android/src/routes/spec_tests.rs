//! Conformance to the shared `android-http-contract.json`: every case is
//! sent as raw bytes to a real bridge; valid frames must reach upstream
//! with the declared body byte-identical; invalid frames get the static
//! 409 `BAD_REQUEST` envelope and never touch upstream/lifecycle.
//! Captured error fixtures are rebuilt through `error_envelope` and
//! checked against the manifest.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use agent_mobile_core::contract::PROTOCOL_VERSION;
use agent_mobile_core::error::Failure;
use serde_json::Value;

use super::status_for_code;
use crate::driver::SecretToken;
use crate::http::start_bridge;
use crate::proxy::{BODY_CAP, HEAD_CAP, error_envelope, read_request};
use crate::testkit::{ctl, read_http_request};

const SPEC_JSON: &str = include_str!("../../../core/tests/spec/android-http-contract.json");
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/");

fn local(msg: impl std::fmt::Display) -> Failure {
    Failure::local(msg.to_string(), "fix the contract test")
}

fn spec() -> Result<Value, Failure> {
    serde_json::from_str(SPEC_JSON).map_err(local)
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, Failure> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| local(format!("missing {key}")))
}

fn expected_body(case: &Value) -> &str {
    case.get("expected_body")
        .or_else(|| case.get("body"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Recording upstream owned by the test: bounded IO, explicit shutdown
/// wake + join so rejected rows leak no listener or thread.
struct ScopedUpstream {
    port: u16,
    /// Hold the binding until shutdown wakes the owned accept loop.
    _listener: TcpListener,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    handle: Option<JoinHandle<()>>,
}

fn accept_loop(
    listener: &TcpListener,
    reply: &str,
    flag: &AtomicBool,
    bucket: &Mutex<Vec<Vec<u8>>>,
) {
    while !flag.load(Ordering::Relaxed) {
        let Ok((mut sock, _)) = listener.accept() else {
            break;
        };
        if flag.load(Ordering::Relaxed) {
            drop(sock);
            break;
        }
        serve_one(&mut sock, reply, bucket);
    }
}

fn serve_one(sock: &mut TcpStream, reply: &str, bucket: &Mutex<Vec<Vec<u8>>>) {
    let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = sock.set_write_timeout(Some(Duration::from_secs(5)));
    let got = read_http_request(sock);
    if let Ok(mut v) = bucket.lock() {
        v.push(got);
    }
    let _ = sock.write_all(reply.as_bytes());
    let _ = sock.shutdown(std::net::Shutdown::Both);
}

impl ScopedUpstream {
    fn new(reply: String) -> Result<Self, Failure> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(local)?;
        let port = listener.local_addr().map_err(local)?.port();
        let owned_listener = listener.try_clone().map_err(local)?;
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let (flag, bucket) = (stop.clone(), seen.clone());
        let handle = thread::spawn(move || accept_loop(&listener, &reply, &flag, &bucket));
        Ok(Self {
            port,
            _listener: owned_listener,
            stop,
            seen,
            handle: Some(handle),
        })
    }

    fn seen(&self) -> Vec<Vec<u8>> {
        self.seen.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

impl Drop for ScopedUpstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Raw request bytes per the spec formula: `request_line + CRLF +
/// base_headers + framing_headers + CRLF + body`. `head_bytes` cases pad
/// `X-Pad: {pad}` with ASCII 'a' so the whole head (through the final
/// CRLFCRLF) is exactly that many bytes. The wire body is always the
/// case's `body` bytes; `expected_body` is assertion-only.
fn raw_request(spec: &Value, case: &Value) -> Result<Vec<u8>, Failure> {
    let request_line = match case.get("request_line").and_then(Value::as_str) {
        Some(line) => line.to_owned(),
        None => text(spec, "request_line")?.to_owned(),
    };
    let mut head = format!(
        "{request_line}\r\n{}{}\r\n",
        text(spec, "base_headers")?,
        text(case, "framing_headers")?
    );
    if let Some(target) = case.get("head_bytes").and_then(Value::as_u64) {
        let unpadded = head.replacen("{pad}", "", 1);
        let pad_len = usize::try_from(target)
            .unwrap_or(0)
            .saturating_sub(unpadded.len());
        head = unpadded.replacen("X-Pad: ", &format!("X-Pad: {}", "a".repeat(pad_len)), 1);
        assert_eq!(
            head.len(),
            usize::try_from(target).unwrap_or(0),
            "head pad math"
        );
    }
    let mut raw = head.into_bytes();
    raw.extend_from_slice(text(case, "body")?.as_bytes());
    Ok(raw)
}

/// Send `raw`, read the reply as a framed HTTP response (bounded):
/// head <=64KiB ending in CRLFCRLF, exactly one numeric Content-Length
/// <=64MiB, then exactly the declared body bytes.
fn round_trip(port: u16, raw: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).map_err(local)?;
    sock.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(local)?;
    sock.set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(local)?;
    sock.write_all(raw).map_err(local)?;
    let _ = sock.shutdown(std::net::Shutdown::Write);
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    while !out.ends_with(b"\r\n\r\n") {
        if out.len() >= 64 * 1024 {
            return Err(local("reply head exceeds cap"));
        }
        if sock.read_exact(&mut byte).is_err() {
            return Err(local("EOF inside reply head"));
        }
        out.push(byte[0]);
    }
    let declared = {
        let text = String::from_utf8_lossy(&out);
        let lens = text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .filter(|(n, _)| n.trim().eq_ignore_ascii_case("content-length"))
            .map(|(_, v)| v.trim().to_owned())
            .collect::<Vec<_>>();
        if lens.len() != 1 || lens[0].is_empty() || !lens[0].bytes().all(|b| b.is_ascii_digit()) {
            return Err(local("reply needs exactly one numeric Content-Length"));
        }
        let n: u64 = lens[0].parse().map_err(local)?;
        if n > 64 * 1024 * 1024 {
            return Err(local("reply Content-Length too large"));
        }
        usize::try_from(n).map_err(local)?
    };
    let head_len = out.len();
    out.resize(head_len + declared, 0);
    sock.read_exact(&mut out[head_len..])
        .map_err(|_| local("reply body truncated"))?;
    Ok(out)
}

fn reply_json(raw: &[u8]) -> Result<(u16, Value), Failure> {
    let text = String::from_utf8_lossy(raw);
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| local("no status in reply"))?;
    let pos = text
        .find("\r\n\r\n")
        .ok_or_else(|| local("no reply terminator"))?;
    let body = serde_json::from_str(&text[pos + 4..]).map_err(local)?;
    Ok((status, body))
}

fn body_of(raw: &[u8]) -> Result<&[u8], Failure> {
    let pos = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| local("no header terminator"))?;
    Ok(&raw[pos + 4..])
}

#[test]
fn spec_metadata_and_error_fixtures_match_production() -> Result<(), Failure> {
    let spec = spec()?;
    assert_eq!(spec["version"].as_str(), Some(PROTOCOL_VERSION));
    assert_eq!(spec["head_cap"].as_u64(), Some(HEAD_CAP as u64));
    assert_eq!(spec["body_cap"].as_u64(), Some(BODY_CAP as u64));
    let cases = spec["cases"].as_array().cloned().unwrap_or_default();
    assert!(!cases.is_empty(), "spec cases must not be empty");
    for guard in ["chunked", "duplicate_content_length"] {
        assert!(
            cases
                .iter()
                .any(|c| c["name"].as_str().unwrap_or("").contains(guard)),
            "missing {guard} guard case"
        );
    }
    let fixtures = spec["error_fixtures"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(6, fixtures.len(), "six captured fixtures required");
    for f in fixtures {
        let file = text(&f, "file")?;
        let raw = std::fs::read_to_string(format!("{FIXTURE_DIR}{file}"))
            .map_err(|e| local(format!("fixture {file}: {e}")))?;
        let captured: Value = serde_json::from_str(&raw).map_err(local)?;
        let code = text(&captured["error"], "code")?;
        let message = text(&captured["error"], "message")?;
        let command = captured["command"].as_str();
        let elapsed = captured["elapsed_ms"].as_u64();
        let rebuilt: Value = serde_json::from_str(&error_envelope(command, elapsed, code, message))
            .map_err(local)?;
        assert_eq!(rebuilt, captured, "envelope for {file}");
        let status_num: u64 = status_for_code(code)
            .split(' ')
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| local("unparseable status"))?;
        assert_eq!(
            status_num,
            f["status"].as_u64().unwrap_or(0),
            "status for {file}"
        );
    }
    Ok(())
}

/// `reads_only_declared_body` sends `{}ignored` on the wire; the parser
/// must keep only the declared `Content-Length` bytes for the body.
#[test]
fn raw_wire_body_is_full_bytes_not_expected_body() -> Result<(), Failure> {
    let spec = spec()?;
    let case = spec["cases"]
        .as_array()
        .and_then(|cases| {
            cases
                .iter()
                .find(|c| c["name"] == "reads_only_declared_body")
        })
        .ok_or_else(|| local("missing reads_only_declared_body case"))?;
    let raw = raw_request(&spec, case)?;
    assert!(body_of(&raw)?.starts_with(b"{}ignored"), "wire bytes");
    let mut slice = raw.as_slice();
    let req = read_request(&mut slice).map_err(local)?;
    assert_eq!(req.body, b"{}", "declared body");
    Ok(())
}

fn run_case(spec: &Value, case: &Value, reply_body: &str) -> Result<(), Failure> {
    let name = text(case, "name")?;
    let upstream = ScopedUpstream::new(format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{reply_body}",
        reply_body.len(),
    ))?;
    let mut bridge = start_bridge(
        upstream.port,
        &SecretToken::new(text(spec, "token")?),
        ctl(),
    )?;
    let raw = raw_request(spec, case)?;
    let reply = round_trip(bridge.port(), &raw)?;
    let (status, body) = reply_json(&reply)?;
    assert_eq!(
        u64::from(status),
        case["status"].as_u64().unwrap_or(0),
        "{name}: status"
    );
    if case["handler_calls"].as_u64().unwrap_or(0) == 1 {
        let seen = upstream.seen();
        assert_eq!(seen.len(), 1, "{name}: upstream saw {seen:?}");
        assert_eq!(
            body_of(&seen[0])?,
            expected_body(case).as_bytes(),
            "{name}: body"
        );
        assert_eq!(body["ok"], true, "{name}: upstream reply ok");
        assert_eq!(body["version"].as_str(), Some(PROTOCOL_VERSION));
    } else {
        assert!(
            upstream.seen().is_empty(),
            "{name}: rejected frame reached upstream"
        );
        assert_eq!(body["ok"], false, "{name}: envelope ok");
        assert_eq!(body["version"].as_str(), Some(PROTOCOL_VERSION));
        if let Some(code) = case.get("error_code").and_then(Value::as_str) {
            assert_eq!(body["error"]["code"].as_str(), Some(code), "{name}: code");
        }
    }
    bridge.stop();
    Ok(())
}

#[test]
fn every_case_drives_the_real_bridge() -> Result<(), Failure> {
    let spec = spec()?;
    let cases = spec["cases"].as_array().cloned().unwrap_or_default();
    assert!(!cases.is_empty());
    let status_raw = std::fs::read_to_string(format!("{FIXTURE_DIR}android-status.json"))
        .map_err(|e| local(format!("fixture: {e}")))?;
    let captured_status = status_raw.trim();
    for case in &cases {
        run_case(&spec, case, captured_status)?;
    }
    Ok(())
}

/// Parser-level: `read_request` verdict must agree with `valid_frame`
/// and preserve the declared body exactly.
#[test]
fn read_request_verdict_and_body_match_spec() -> Result<(), Failure> {
    let spec = spec()?;
    for case in spec["cases"].as_array().cloned().unwrap_or_default() {
        let raw = raw_request(&spec, &case)?;
        let mut slice = raw.as_slice();
        let parsed = read_request(&mut slice);
        assert_eq!(
            parsed.is_ok(),
            case["valid_frame"].as_bool().unwrap_or(false),
            "parse verdict for {}",
            text(&case, "name")?
        );
        if let Ok(req) = parsed {
            assert_eq!(
                req.body,
                expected_body(&case).as_bytes(),
                "body for {}",
                text(&case, "name")?
            );
        }
    }
    Ok(())
}
