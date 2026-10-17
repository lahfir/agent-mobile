//! One serial worker; abandoning a response never repeats an action.

use std::sync::mpsc::{Receiver, Sender};

use agent_mobile_core::contract::Data;
use agent_mobile_core::error::Failure;
use agent_mobile_core::state::StateStore;
use serde_json::{Value, json};

use crate::cmd::{self, Ctx};

use super::{Event, frame, tools};

/// Server-wide CLI selectors; tool arguments cannot change the device.
pub struct Options {
    pub device: Option<String>,
    pub app: Option<String>,
    pub max_depth: Option<u32>,
}

/// One validated call, identified independently of client-supplied ids.
pub struct Job {
    pub sequence: u64,
    pub call: tools::Call,
}

/// ponytail: one worker preserves call ordering without an async runtime.
pub fn worker(jobs: Receiver<Job>, events: &Sender<Event>, options: &Options) {
    for job in jobs {
        let sequence = job.sequence;
        let result = execute(job, events, options)
            .unwrap_or_else(|failure| frame::text(&frame::redact(failure.render()), true));
        if events.send(Event::Done(sequence, result)).is_err() {
            break;
        }
    }
}

fn execute(job: Job, events: &Sender<Event>, options: &Options) -> Result<Value, Failure> {
    let call = job.call;
    if call.name == "skills" {
        return Ok(frame::text(cmd::skills::reply(), false));
    }
    if call.name == "devices" {
        let scan = crate::platform::discover()?;
        let text = cmd::devices::reply(&scan.devices, &StateStore::new()?.load(), false)?;
        return Ok(frame::text(&frame::redact(text), false));
    }
    let _ = events.send(Event::Boot(job.sequence, true));
    let ctx = Ctx::from_options(
        options.device.as_deref(),
        call.app.clone().or_else(|| options.app.clone()),
        call.max_depth.or(options.max_depth),
    )?;
    let session = ctx.session()?;
    let _ = events.send(Event::Boot(job.sequence, false));
    if call.name == "scroll_until_visible" {
        let request = call
            .request
            .ok_or_else(|| Failure::usage("missing request"))?;
        let text = request.body["text"]
            .as_str()
            .ok_or_else(|| Failure::usage("missing text"))?;
        let direction = request.body["direction"].as_str().unwrap_or("up");
        let env = cmd::scroll::reply(&ctx, text, direction)?;
        return Ok(frame::text(&frame::redact(ctx.reply(env)?), false));
    }
    let mut request = call
        .request
        .ok_or_else(|| Failure::usage("missing request"))?;
    if call.name == "snapshot" && call.app.is_none() {
        request = cmd::snapshot::request(options.app.as_deref());
    }
    let env = request.call(&session)?;
    if call.name == "screenshot" && env.ok {
        if let Some(Data::Screenshot(shot)) = env.data {
            return Ok(
                json!({"content":[{"type":"image", "mimeType":"image/png", "data":shot.png_base64}], "isError":false}),
            );
        }
        return Err(Failure::local("missing screenshot payload", "report a bug"));
    }
    Ok(frame::text(&frame::redact(ctx.reply(env)?), false))
}

/// Timeout recovery depends on whether the worker is booting or executing.
pub fn timeout(name: &str, boot: bool) -> Value {
    let next = if !tools::read_only(name) {
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
