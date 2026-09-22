//! `home`: press Home and return the springboard tree.

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `home`; one round trip.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(ctx, &session, "home", &serde_json::json!({}))
}
