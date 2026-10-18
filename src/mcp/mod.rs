//! Stdio-only MCP: a responsive protocol loop around one blocking worker.

mod call;
mod frame;
mod protocol;
mod rpc;

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use serde_json::{Value, json};

use crate::cmd::Call;
use crate::surface::mcp::{Rejected, prepare, tools};

enum Event {
    Line(Vec<u8>),
    Eof,
    Boot(u64, bool),
    Done(u64, Value),
}

struct Pending {
    id: Value,
    modern: bool,
    deadline: Instant,
    name: String,
    boot: bool,
}

struct Server {
    legacy: bool,
    sequence: u64,
    budget: Duration,
    pending: BTreeMap<u64, Pending>,
    jobs: Sender<call::Job>,
}

/// Run until stdin EOF or the client closes stdout; failures stay on stderr.
pub fn run(call: &Call) -> Result<i32, Failure> {
    let (events, input) = mpsc::channel();
    let reader = events.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().split(b'\n') {
            let Ok(line) = line else { break };
            if reader.send(Event::Line(line)).is_err() {
                return;
            }
        }
        let _ = reader.send(Event::Eof);
    });
    let (jobs, work) = mpsc::channel();
    let defaults = call.args.clone();
    std::thread::spawn(move || call::worker(work, &events, &defaults));
    let budget = std::env::var("AGENT_MOBILE_MCP_CALL_SECS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 45.0)
        .map_or(Duration::from_secs(45), Duration::from_secs_f64);
    let mut server = Server {
        legacy: false,
        sequence: 0,
        budget,
        pending: BTreeMap::new(),
        jobs,
    };
    let result = server.listen(&input, &mut std::io::stdout().lock());
    match result {
        Ok(()) => Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(0),
        Err(error) => Err(error.into()),
    }
}

impl Server {
    fn listen(&mut self, events: &Receiver<Event>, out: &mut impl Write) -> std::io::Result<()> {
        loop {
            self.expire(out)?;
            let wait = self
                .pending
                .values()
                .map(|p| p.deadline.saturating_duration_since(Instant::now()))
                .min()
                .unwrap_or(Duration::from_secs(3600));
            match events.recv_timeout(wait) {
                Ok(Event::Line(line)) => self.line(&line, out)?,
                Ok(Event::Boot(sequence, boot)) => {
                    self.boot(sequence, boot);
                }
                Ok(Event::Done(sequence, result)) => {
                    self.done(sequence, result, out)?;
                }
                Ok(Event::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn boot(&mut self, sequence: u64, boot: bool) {
        if let Some(pending) = self.pending.get_mut(&sequence) {
            pending.boot = boot;
        }
    }

    fn done(&mut self, sequence: u64, result: Value, out: &mut impl Write) -> std::io::Result<()> {
        if let Some(pending) = self.pending.remove(&sequence) {
            rpc::write(
                out,
                &rpc::success(&pending.id, &protocol::complete(result, pending.modern)),
            )?;
        }
        Ok(())
    }

    fn expire(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        let now = Instant::now();
        let expired: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= now)
            .map(|(seq, _)| *seq)
            .collect();
        for sequence in expired {
            if let Some(p) = self.pending.remove(&sequence) {
                let result = protocol::complete(call::timeout(&p.name, p.boot), p.modern);
                rpc::write(out, &rpc::success(&p.id, &result))?;
            }
        }
        Ok(())
    }

    fn line(&mut self, line: &[u8], out: &mut impl Write) -> std::io::Result<()> {
        let request = match rpc::parse(line) {
            Ok(request) => request,
            Err(error) => return rpc::write(out, &error),
        };
        let Some(id) = request.id.clone() else {
            if request.method == "notifications/cancelled" {
                self.pending
                    .retain(|_, p| p.id != request.params["requestId"]);
            }
            return Ok(());
        };
        let modern = match protocol::modern(&request, self.legacy) {
            Ok(modern) => modern,
            Err(error) => return rpc::write(out, &error),
        };
        let result = match request.method.as_str() {
            "server/discover" => protocol::discover(),
            "initialize" => {
                let Some(result) = protocol::initialize(&request.params) else {
                    return rpc::write(
                        out,
                        &rpc::error(&id, -32602, "Invalid initialize parameters"),
                    );
                };
                self.legacy = true;
                result
            }
            "ping" if !modern => json!({}),
            "tools/list" => tools(),
            "tools/call" => {
                let call = match prepare(&request.params) {
                    Ok(call) => call,
                    Err(Rejected::Protocol(message)) => {
                        return rpc::write(out, &rpc::error(&id, -32602, message));
                    }
                    Err(Rejected::Tool(fault)) => {
                        let result = frame::text(&fault.tool_text(), true);
                        return rpc::write(
                            out,
                            &rpc::success(&id, &protocol::complete(result, modern)),
                        );
                    }
                };
                if self.pending.values().any(|p| p.id == id) {
                    return rpc::write(out, &rpc::error(&id, -32600, "Duplicate pending id"));
                }
                self.sequence += 1;
                let name = call.verb.name.to_owned();
                self.jobs
                    .send(call::Job {
                        sequence: self.sequence,
                        call,
                    })
                    .map_err(|_| std::io::Error::other("MCP worker stopped"))?;
                self.pending.insert(
                    self.sequence,
                    Pending {
                        id,
                        modern,
                        name,
                        deadline: Instant::now() + self.budget,
                        boot: false,
                    },
                );
                return Ok(());
            }
            _ => return rpc::write(out, &rpc::error(&id, -32601, "Method not found")),
        };
        rpc::write(out, &rpc::success(&id, &protocol::complete(result, modern)))
    }
}
