//! `scroll_until_visible`: bounded host-side snapshot and swipe loop.

use std::time::{Duration, Instant};

use agent_mobile_core::contract::{Data, Node};
use agent_mobile_core::error::{ErrorCode, Failure};

use super::{Ctx, reply::Request};

const MAX_SWIPES: usize = 15;
const TIME_BOUND: Duration = Duration::from_secs(40);

/// Validate and build the swipe request used for each scroll step.
pub fn request(direction: &str) -> Result<Request, Failure> {
    if !matches!(direction, "up" | "down" | "left" | "right") {
        return Err(Failure::usage(
            "scroll_until_visible direction must be up, down, left, or right",
        ));
    }
    super::swipe::request(direction, None)
}

fn names(node: &Node, list: &mut Vec<(String, String)>) {
    list.push((node.role.clone(), node.name.clone()));
    for child in &node.children {
        names(child, list);
    }
}

fn snapshot(
    env: &agent_mobile_core::contract::Envelope,
) -> Result<&agent_mobile_core::contract::Snapshot, Failure> {
    if !env.ok {
        return Err(env.error.as_ref().map_or_else(
            || Failure::local("empty error envelope", "report a bug"),
            Failure::from_error_body,
        ));
    }
    match env.data.as_ref() {
        Some(Data::Snapshot(value)) => Ok(value.as_ref()),
        _ => Err(Failure::local("missing snapshot payload", "report a bug")),
    }
}

fn contains(node: &Node, target: &str) -> bool {
    node.name == target || node.children.iter().any(|child| contains(child, target))
}

/// Run the bounded loop and return the rendered final snapshot.
pub fn reply(ctx: &Ctx, text: &str, direction: &str) -> Result<String, Failure> {
    let request = request(direction)?;
    let session = ctx.session()?;
    let deadline = Instant::now() + TIME_BOUND;
    let mut initial = super::snapshot::request(ctx.app_selector());
    initial.timeout = Some(TIME_BOUND);
    let mut current = initial.call(&session)?;
    if contains(&snapshot(&current)?.tree, text) {
        return ctx.reply(current);
    }
    for _ in 0..MAX_SWIPES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Failure::driver(
                ErrorCode::DriverError,
                "not found within scroll time bound",
            ));
        }
        let before = snapshot(&current)?;
        let mut node_list = Vec::new();
        names(&before.tree, &mut node_list);
        let mut step = Request::new(request.verb, request.body.clone());
        step.timeout = Some(remaining);
        current = step.call(&session)?;
        let after = snapshot(&current)?;
        if contains(&after.tree, text) {
            return ctx.reply(current);
        }
        let mut next_list = Vec::new();
        names(&after.tree, &mut next_list);
        if node_list == next_list {
            return Err(Failure::driver(
                ErrorCode::DriverError,
                "not found before end of list",
            ));
        }
    }
    Err(Failure::driver(
        ErrorCode::DriverError,
        "not found within 15 swipes",
    ))
}

/// CLI wrapper.
pub fn run(ctx: &Ctx, text: &str, direction: &str) -> Result<i32, Failure> {
    let rendered = reply(ctx, text, direction)?;
    super::emit(&rendered);
    Ok(0)
}
