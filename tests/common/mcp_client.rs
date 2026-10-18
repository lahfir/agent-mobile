//! The MCP stdio test client: one `agent-mobile mcp` child per test.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout};

use agent_mobile_core::error::Failure;
use serde_json::{Value, json};

pub struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl Client {
    pub fn new(envs: &[(&str, &str)]) -> Result<Self, Failure> {
        Self::at(&crate::common::tmp_home("mcp")?, envs)
    }

    pub fn at(home: &PathBuf, envs: &[(&str, &str)]) -> Result<Self, Failure> {
        let mut child = crate::common::spawn_stdio(&["--json", "mcp"], home, envs)?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| crate::common::fail("stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| crate::common::fail("stdout"))?;
        Ok(Self {
            child,
            input: Some(input),
            output: BufReader::new(output),
        })
    }

    pub fn send(&mut self, value: &Value) -> Result<(), Failure> {
        self.line(value.to_string().as_bytes())
    }

    pub fn line(&mut self, line: &[u8]) -> Result<(), Failure> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| crate::common::fail("closed stdin"))?;
        input.write_all(line)?;
        input.write_all(b"\n")?;
        input.flush()?;
        Ok(())
    }

    pub fn read(&mut self) -> Result<Value, Failure> {
        let mut line = String::new();
        assert!(self.output.read_line(&mut line)? > 0, "unexpected EOF");
        serde_json::from_str(&line).map_err(|error| crate::common::fail(&error.to_string()))
    }

    pub fn request(&mut self, id: i64, method: &str, params: &Value) -> Result<Value, Failure> {
        self.send(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))?;
        let reply = self.read()?;
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], id);
        Ok(reply)
    }

    pub fn legacy(&mut self) -> Result<Value, Failure> {
        let reply = self.request(
            0,
            "initialize",
            &json!({
                "protocolVersion":"2025-11-25", "capabilities":{},
                "clientInfo":{"name":"test", "version":"1"}
            }),
        )?;
        self.send(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))?;
        Ok(reply)
    }

    pub fn finish(mut self) -> Result<(), Failure> {
        self.input.take();
        let mut extra = String::new();
        assert_eq!(self.output.read_line(&mut extra)?, 0, "{extra}");
        assert!(self.child.wait()?.success());
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
