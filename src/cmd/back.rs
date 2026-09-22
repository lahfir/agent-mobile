//! `back`: perform the system edge swipe back; no target, empty body.

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `back`; one round trip.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(ctx, &session, "back", &serde_json::json!({}))
}
