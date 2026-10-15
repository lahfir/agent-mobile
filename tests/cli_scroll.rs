//! `scroll_until_visible` CLI and MCP behavior against a stub driver.

#[allow(dead_code, reason = "shared CLI and driver fixtures")]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout};

use agent_mobile_core::error::Failure;
use serde_json::{Value, json};

use common::{code, run_wired, stderr, stdout, stub, tmp_home};

fn snapshot(id: &str, names: &[&str]) -> String {
    let children: Vec<Value> = names
        .iter()
        .map(|name| {
            json!({
                "role":"button", "name":name, "value":"", "ref_id":format!("@{id}:e2"),
                "states":[], "available_actions":["Tap"],
                "bounds":{"x":0.0,"y":0.0,"width":100.0,"height":50.0}, "children":[]
            })
        })
        .collect();
    let tree = json!({
        "role":"application", "name":"App", "value":"", "ref_id":format!("@{id}:e1"),
        "states":[], "available_actions":[], "bounds":{"x":0.0,"y":0.0,"width":430.0,"height":930.0},
        "children":children
    });
    let names_text = names.join(" ");
    json!({
        "version":"1", "ok":true, "command":"snapshot", "elapsed_ms":1,
        "data":{"app":"com.x", "snapshot_id":id, "ref_count":names.len()+1, "complete":true,
        "settled":true, "reads":3, "text":names_text, "tree":tree}
    })
    .to_string()
}

#[test]
fn finds_visible_name_after_two_swipes() -> Result<(), Failure> {
    let first = snapshot("s1", &["Top"]);
    let second = snapshot("s2", &["Middle"]);
    let third = snapshot("s3", &["About"]);
    let stub = stub(&[&first, &second, &third])?;
    let home = tmp_home("scroll-found")?;
    let out = run_wired(&["scroll_until_visible", "About"], &home, &stub)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("About"));
    let calls = stub.captured()?;
    assert_eq!(calls.len(), 3);
    assert!(calls[0].starts_with("POST /snapshot "));
    assert!(calls[1].starts_with("POST /swipe "));
    assert!(calls[2].starts_with("POST /swipe "));
    Ok(())
}

#[test]
fn unchanged_tree_fails_after_one_swipe() -> Result<(), Failure> {
    let first = snapshot("s1", &["Top"]);
    let second = snapshot("s2", &["Top"]);
    let stub = stub(&[&first, &second])?;
    let home = tmp_home("scroll-end")?;
    let out = run_wired(&["scroll_until_visible", "Missing"], &home, &stub)?;
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    assert!(stderr(&out).contains("DRIVER_ERROR"));
    assert!(stderr(&out).contains("not found before end of list"));
    assert_eq!(stub.captured()?.len(), 2);
    Ok(())
}

#[test]
fn already_visible_returns_without_swiping() -> Result<(), Failure> {
    let first = snapshot("s1", &["About"]);
    let stub = stub(&[&first])?;
    let home = tmp_home("scroll-visible")?;
    let out = run_wired(&["scroll_until_visible", "About"], &home, &stub)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("About"));
    let calls = stub.captured()?;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("POST /snapshot "));
    Ok(())
}

#[test]
fn invalid_direction_fails_without_wire_call() -> Result<(), Failure> {
    let stub = stub(&[])?;
    let home = tmp_home("scroll-direction")?;
    let out = run_wired(&["scroll_until_visible", "About", "diagonal"], &home, &stub)?;
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("invalid value"));
    assert!(stub.captured()?.is_empty());
    Ok(())
}

struct McpClient {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl McpClient {
    fn new(home: &std::path::Path, envs: &[(&str, &str)]) -> Result<Self, Failure> {
        let mut child = common::spawn_stdio(&["--json", "mcp"], &home.to_path_buf(), envs)?;
        let input = child.stdin.take().ok_or_else(|| common::fail("stdin"))?;
        let output = child.stdout.take().ok_or_else(|| common::fail("stdout"))?;
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
        })
    }

    fn exchange(&mut self, value: &Value) -> Result<Value, Failure> {
        writeln!(self.input, "{value}")?;
        self.input.flush()?;
        let mut line = String::new();
        assert!(self.output.read_line(&mut line)? > 0);
        serde_json::from_str(&line).map_err(|error| common::fail(&error.to_string()))
    }

    fn notify(&mut self, value: &Value) -> Result<(), Failure> {
        writeln!(self.input, "{value}")?;
        self.input.flush()?;
        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_found_case_matches_cli_text() -> Result<(), Failure> {
    let first = snapshot("s1", &["Top"]);
    let second = snapshot("s2", &["About"]);
    let cli_stub = stub(&[&first, &second])?;
    let cli_home = tmp_home("scroll-parity-cli")?;
    let cli = run_wired(&["scroll_until_visible", "About"], &cli_home, &cli_stub)?;
    assert_eq!(code(&cli), 0, "{}", stderr(&cli));
    let expected = stdout(&cli).trim().to_owned();
    assert_eq!(cli_stub.captured()?.len(), 2);

    let mcp_stub = stub(&[&first, &second])?;
    let home = tmp_home("scroll-parity-mcp")?;
    let mut client = McpClient::new(
        &home,
        &[
            ("AGENT_MOBILE_URL", &mcp_stub.url),
            ("AGENT_MOBILE_TOKEN", "tok"),
        ],
    )?;
    let init = client.exchange(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-11-25", "capabilities":{}, "clientInfo":{"name":"test","version":"1"}}}))?;
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    client.notify(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))?;
    let reply = client.exchange(&json!({"jsonrpc":"2.0", "id":2, "method":"tools/call", "params":{"name":"scroll_until_visible", "arguments":{"text":"About"}}}))?;
    assert_eq!(reply["result"]["isError"], false);
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| common::fail("missing tool text"))?;
    assert_eq!(
        text.strip_prefix("<untrusted-data>\n")
            .and_then(|value| value.strip_suffix("\n</untrusted-data>"))
            .map(str::trim_end),
        Some(expected.as_str())
    );
    assert_eq!(mcp_stub.captured()?.len(), 2);
    Ok(())
}
