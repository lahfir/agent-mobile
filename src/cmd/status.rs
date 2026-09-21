//! `status`: the driver's identity reply — app, device, os, snapshot id.

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `status`; one round trip.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(ctx, &session, "status", &serde_json::json!({}))
}
