//! End-to-end CLI tests: the built binary against stub HTTP servers, one
//! isolated HOME per test, covering the argument shapes, output contract,
//! and exit codes.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn fail(msg: &str) -> Failure {
    Failure::local(msg.to_owned(), "fix the test")
}

fn tmp_home(tag: &str) -> Result<PathBuf, Failure> {
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
struct Stub {
    url: String,
    join: JoinHandle<Vec<String>>,
}

fn stub(bodies: &[&str]) -> Result<Stub, Failure> {
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

fn run(args: &[&str], home: &PathBuf, envs: &[(&str, &str)]) -> Result<Output, Failure> {
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

fn run_wired(args: &[&str], home: &PathBuf, s: &Stub) -> Result<Output, Failure> {
    run(
        args,
        home,
        &[
            ("AGENT_MOBILE_URL", s.url.as_str()),
            ("AGENT_MOBILE_TOKEN", "tok"),
        ],
    )
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn body_of(req: &str) -> Result<serde_json::Value, Failure> {
    let body = req
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| fail("request has no body"))?;
    serde_json::from_str(body).map_err(|e| fail(&format!("bad json body: {e}")))
}

const STATUS: &str = r#"{"version":"1","ok":true,"command":"status","elapsed_ms":3,"data":{"app":"com.apple.springboard","device":"Sim","os":"26.0","snapshot_id":"sid1"}}"#;

const SNAPSHOT: &str = r#"{"version":"1","ok":true,"command":"snapshot","elapsed_ms":7,"data":{"app":"com.x","snapshot_id":"snap1","ref_count":2,"complete":true,"settled":true,"reads":2,"text":"t","tree":{"role":"application","name":"App","value":"","ref_id":"@snap1:e1","states":[],"available_actions":[],"bounds":{"x":0.0,"y":0.0,"width":430.0,"height":930.0},"children":[{"role":"group","name":"Outer","value":"","ref_id":"@snap1:e2","states":[],"available_actions":["Tap"],"bounds":{"x":0.0,"y":0.0,"width":100.0,"height":50.0},"children":[{"role":"button","name":"Deep","value":"","ref_id":"@snap1:e3","states":[],"available_actions":["Tap"],"bounds":{"x":1.0,"y":2.0,"width":10.0,"height":10.0},"children":[]}]}]}}}"#;

const STALE: &str = r#"{"version":"1","ok":false,"command":"tap","elapsed_ms":4,"error":{"code":"STALE_REF","message":"ref gone"}}"#;

#[test]
fn tap_one_arg_sends_ref() -> Result<(), Failure> {
    let home = tmp_home("tap-ref")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["tap", "@abc:e1"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["ref"], "@abc:e1");
    assert!(v.get("x").is_none());
    Ok(())
}

#[test]
fn tap_two_args_send_float_coordinates() -> Result<(), Failure> {
    let home = tmp_home("tap-xy")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["tap", "10.5", "20"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["x"], 10.5);
    assert_eq!(v["y"], 20.0);
    assert!(v.get("ref").is_none());
    Ok(())
}

#[test]
fn tap_with_no_args_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("tap-none")?;
    let out = run(&["tap"], &home, &[])?;
    assert_eq!(code(&out), 2);
    Ok(())
}

#[test]
fn tap_bad_coordinate_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("tap-bad")?;
    let out = run(&["tap", "10", "abc"], &home, &[])?;
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("x y"), "{}", stderr(&out));
    Ok(())
}

#[test]
fn type_consumes_leading_ref_then_joins_text() -> Result<(), Failure> {
    let home = tmp_home("type-ref")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["type", "@abc:e1", "hello", "world"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["ref"], "@abc:e1");
    assert_eq!(v["text"], "hello world");
    Ok(())
}

#[test]
fn type_at_mention_is_text_not_ref() -> Result<(), Failure> {
    let home = tmp_home("type-at")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["type", "@handle", "hi"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert!(v.get("ref").is_none(), "{v}");
    assert_eq!(v["text"], "@handle hi");
    Ok(())
}

#[test]
fn type_ref_without_text_is_usage_error() -> Result<(), Failure> {
    let home = tmp_home("type-bare")?;
    let out = run(&["type", "@abc:e1"], &home, &[])?;
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("text"), "{}", stderr(&out));
    Ok(())
}

#[test]
fn swipe_rejects_bad_direction_before_network() -> Result<(), Failure> {
    let home = tmp_home("swipe-bad")?;
    let out = run(&["swipe", "diagonal"], &home, &[])?;
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("up"), "{err}");
    assert!(err.contains("down"), "{err}");
    Ok(())
}

#[test]
fn swipe_direction_and_ref_reach_body() -> Result<(), Failure> {
    let home = tmp_home("swipe-ok")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["swipe", "left", "@abc:e2"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["direction"], "left");
    assert_eq!(v["ref"], "@abc:e2");
    Ok(())
}

#[test]
fn json_flag_passes_the_envelope() -> Result<(), Failure> {
    let home = tmp_home("json-ok")?;
    let s = stub(&[STATUS])?;
    let out = run_wired(&["status", "--json"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let got: serde_json::Value = serde_json::from_str(stdout(&out).trim())
        .map_err(|e| fail(&format!("json stdout: {e}")))?;
    let want: serde_json::Value =
        serde_json::from_str(STATUS).map_err(|e| fail(&format!("fixture: {e}")))?;
    assert_eq!(got, want);
    Ok(())
}

#[test]
fn json_flag_passes_error_envelopes_with_exit_1() -> Result<(), Failure> {
    let home = tmp_home("json-err")?;
    let s = stub(&[STALE])?;
    let out = run_wired(&["tap", "@abc:e1", "--json"], &home, &s)?;
    assert_eq!(code(&out), 1);
    let got: serde_json::Value = serde_json::from_str(stdout(&out).trim())
        .map_err(|e| fail(&format!("json stdout: {e}")))?;
    assert_eq!(got["error"]["code"], "STALE_REF");
    Ok(())
}

#[test]
fn text_error_goes_to_stderr_leaving_stdout_parseable() -> Result<(), Failure> {
    let home = tmp_home("stderr")?;
    let s = stub(&[STALE])?;
    let out = run_wired(&["tap", "@abc:e1"], &home, &s)?;
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).is_empty(), "{}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("STALE_REF"), "{err}");
    assert!(err.contains("next:"), "{err}");
    Ok(())
}

#[test]
fn app_flag_reaches_snapshot_body() -> Result<(), Failure> {
    let home = tmp_home("app")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["snapshot", "--app", "com.foo"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    let v = body_of(captured.first().ok_or_else(|| fail("no request"))?)?;
    assert_eq!(v["app"], "com.foo");
    Ok(())
}

#[test]
fn max_depth_trims_tree_and_marks_incomplete() -> Result<(), Failure> {
    let home = tmp_home("depth")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["snapshot", "--max-depth", "1"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("complete=false"), "{text}");
    assert!(text.contains("Outer"), "{text}");
    assert!(!text.contains("Deep"), "{text}");
    Ok(())
}

#[test]
fn device_flag_is_remembered_and_routes_next_call() -> Result<(), Failure> {
    let home = tmp_home("device")?;
    let s = stub(&[STATUS, STATUS])?;
    let store = StateStore::at(&home.join(".agent-mobile"));
    store.write_token("sim", "tok")?;
    store.upsert(
        "sim",
        &SessionEntry::new(s.url.clone(), 4242, "sim".to_owned()),
    )?;
    let out = run(&["status", "--device", "sim"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = run(&["status"], &home, &[])?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let captured = s.join.join().map_err(|_| fail("stub join"))?;
    assert_eq!(captured.len(), 2, "{captured:?}");
    let state = store.load();
    assert_eq!(state.default_device.as_deref(), Some("sim"));
    Ok(())
}

#[test]
fn no_session_names_the_remedy() -> Result<(), Failure> {
    let home = tmp_home("nosession")?;
    let out = run(&["status"], &home, &[])?;
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(err.contains("serve"), "{err}");
    Ok(())
}

#[test]
fn stale_ref_from_state_fails_before_network() -> Result<(), Failure> {
    let home = tmp_home("localstale")?;
    let store = StateStore::at(&home.join(".agent-mobile"));
    store.write_token("sim", "tok")?;
    store.upsert(
        "sim",
        &SessionEntry::new("http://127.0.0.1:1".to_owned(), 4242, "sim".to_owned()),
    )?;
    store.record_snapshot("sim", "newest")?;
    let out = run(&["tap", "@older:e1", "--device", "sim"], &home, &[])?;
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(err.contains("STALE_REF"), "{err}");
    assert!(err.contains("re-snapshot"), "{err}");
    Ok(())
}

#[test]
fn status_text_output_snapshot() -> Result<(), Failure> {
    let home = tmp_home("snap-status")?;
    let s = stub(&[STATUS])?;
    let out = run_wired(&["status"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    insta::assert_snapshot!(
        stdout(&out).trim_end(),
        @"app=com.apple.springboard device=\"Sim\" os=26.0 snapshot=@sid1 elapsed_ms=3"
    );
    Ok(())
}

#[test]
fn snapshot_text_output_snapshot() -> Result<(), Failure> {
    let home = tmp_home("snap-tree")?;
    let s = stub(&[SNAPSHOT])?;
    let out = run_wired(&["snapshot"], &home, &s)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    insta::assert_snapshot!(stdout(&out), @r###"
    app=com.x snapshot=@snap1 refs=2 settled=true reads=2 elapsed_ms=7
    @snap1:e1 application "App" at=0,0 size=430x930
      @snap1:e2 group "Outer" at=0,0 size=100x50
        @snap1:e3 button "Deep" at=1,2 size=10x10
    "###);
    Ok(())
}
