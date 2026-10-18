//! `scroll_until_visible`: bounded host-side snapshot and swipe loop.

use std::time::{Duration, Instant};

use agent_mobile_core::contract::{Data, Node};
use agent_mobile_core::error::{ErrorCode, Failure};

use super::{Ctx, reply::Request};

const MAX_SWIPES: usize = 15;
const TIME_BOUND: Duration = Duration::from_secs(40);

fn without_refs(node: &mut Node) {
    node.ref_id.clear();
    for child in &mut node.children {
        without_refs(child);
    }
}

fn snapshot(
    env: &agent_mobile_core::contract::Envelope,
) -> Result<&agent_mobile_core::contract::Snapshot, Failure> {
    if !env.ok {
        return Err(super::reply::error_failure(env));
    }
    match env.data.as_ref() {
        Some(Data::Snapshot(value)) => Ok(value.as_ref()),
        _ => Err(Failure::local("missing snapshot payload", "report a bug")),
    }
}

fn contains(node: &Node, target: &str) -> bool {
    node.name == target || node.children.iter().any(|child| contains(child, target))
}

/// Run the bounded loop and return the final snapshot envelope.
pub fn reply(
    ctx: &Ctx,
    text: &str,
    direction: &str,
) -> Result<agent_mobile_core::contract::Envelope, Failure> {
    let swipe = serde_json::json!({ "direction": direction });
    let session = ctx.session()?;
    let deadline = Instant::now() + TIME_BOUND;
    let app = ctx.app_selector().map_or_else(
        || serde_json::json!({}),
        |a| serde_json::json!({ "app": a }),
    );
    let mut initial = Request::new("snapshot", app);
    initial.timeout = Some(TIME_BOUND);
    let mut current = initial.call(&session)?;
    if contains(&snapshot(&current)?.tree, text) {
        return Ok(current);
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
        let mut previous_tree = before.tree.clone();
        without_refs(&mut previous_tree);
        let mut step = Request::new("swipe", swipe.clone());
        step.timeout = Some(remaining);
        current = step.call(&session)?;
        let after = snapshot(&current)?;
        if contains(&after.tree, text) {
            return Ok(current);
        }
        let mut next_tree = after.tree.clone();
        without_refs(&mut next_tree);
        if previous_tree == next_tree {
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

/// The `scroll_until_visible` verb.
///
/// # Errors
/// Not found, or session and transport failures.
pub fn exec(call: &super::Call) -> Result<super::Reply, Failure> {
    let ctx = Ctx::for_call(call)?;
    let text = call.str("text").unwrap_or_default();
    let direction = call.str("direction").unwrap_or("up");
    reply(&ctx, text, direction).map(super::Reply::Envelope)
}
