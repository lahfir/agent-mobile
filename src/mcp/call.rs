//! One serial worker; abandoning a response never repeats an action.

use std::sync::mpsc::{Receiver, Sender};

use agent_mobile_core::contract::{Data, Envelope};
use agent_mobile_core::error::Failure;
use serde_json::{Map, Value, json};

use crate::cmd::{self, Call, Ctx, Reply, reply};
use crate::surface::{self, Global};

use super::{Event, frame};

/// One validated call, identified independently of client-supplied ids.
pub struct Job {
    pub sequence: u64,
    pub call: Call,
}

/// ponytail: one worker preserves call ordering without an async runtime.
/// `defaults` are the server's start-up `--device`/`--app`/`--max-depth`.
pub fn worker(jobs: Receiver<Job>, events: &Sender<Event>, defaults: &Map<String, Value>) {
    for job in jobs {
        let sequence = job.sequence;
        let result = execute(job, events, defaults)
            .unwrap_or_else(|failure| frame::text(&frame::redact(failure.render()), true));
        if events.send(Event::Done(sequence, result)).is_err() {
            break;
        }
    }
}

fn execute(
    job: Job,
    events: &Sender<Event>,
    defaults: &Map<String, Value>,
) -> Result<Value, Failure> {
    let mut call = job.call;
    for global in call.verb.globals {
        let name = global.arg().name;
        if !call.args.contains_key(name)
            && let Some(value) = defaults.get(name)
        {
            call.args.insert(name.to_owned(), value.clone());
        }
    }
    if call.verb.globals.contains(&Global::Device) {
        let _ = events.send(Event::Boot(job.sequence, true));
        Ctx::for_call(&call)?.session()?;
        let _ = events.send(Event::Boot(job.sequence, false));
    }
    render(&call, cmd::execute(&call)?)
}

/// A reply as an MCP tool result: framed text, plus `structuredContent`
/// for every reply kind with an output schema (R11, KTD6).
fn render(call: &Call, reply: Reply) -> Result<Value, Failure> {
    match reply {
        Reply::Envelope(env) => envelope(call, env),
        Reply::Devices { rows, text } => {
            let mut result = frame::text(&frame::redact(text), false);
            result["structuredContent"] = frame::clean(json!({ "devices": rows }));
            Ok(result)
        }
        Reply::Text(text) => Ok(frame::text(&frame::redact(text), false)),
        Reply::Exit(_) => Ok(frame::text("ok", false)),
    }
}

fn envelope(call: &Call, mut env: Envelope) -> Result<Value, Failure> {
    if !env.ok {
        return Err(reply::error_failure(&env));
    }
    reply::trim(&mut env, call.max_depth());
    let structured = match &env.data {
        Some(Data::Screenshot(shot)) => {
            return Ok(
                json!({"content":[{"type":"image", "mimeType":"image/png", "data":shot.png_base64}], "isError":false}),
            );
        }
        Some(Data::Snapshot(s)) => Some(json!({
            "app": s.app, "snapshot_id": s.snapshot_id, "ref_count": s.ref_count,
            "complete": s.complete, "settled": s.settled, "reads": s.reads, "settle_ms": s.settle_ms,
        })),
        Some(Data::Status(s)) => serde_json::to_value(s).ok(),
        Some(Data::Terminate(t)) => serde_json::to_value(t).ok(),
        None => None,
    };
    let text = reply::text(env, None)?;
    let mut result = frame::text(&frame::redact(text), false);
    if surface::mcp::output_schema(call.verb.reply).is_some()
        && let Some(mut structured) = structured
    {
        if let Some(map) = structured.as_object_mut() {
            map.retain(|_, v| !v.is_null());
        }
        result["structuredContent"] = frame::clean(structured);
    }
    Ok(result)
}

/// Timeout recovery depends on whether the worker is booting or executing.
pub fn timeout(name: &str, boot: bool) -> Value {
    let read_only = surface::find(name).is_some_and(|v| v.hints.read_only);
    let next = if !read_only {
        "the action may already have run; take a snapshot before repeating it"
    } else if boot {
        "driver is starting; call again"
    } else {
        "call timed out; take a fresh snapshot and call again"
    };
    frame::text(
        &format!("DRIVER_ERROR: call budget exceeded\nnext: {next}"),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_timeout_does_not_retry_actions() {
        let action = timeout("tap", true);
        assert!(
            action["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("action may already have run"))
        );
        let read = timeout("snapshot", true);
        assert!(
            read["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("driver is starting; call again"))
        );
    }
}
