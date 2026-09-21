//! `stop`: sends `terminate` — the app dies, the driver stays up, and the
//! driver's bundle resets to springboard (KTD10).

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `stop`; one round trip printing the terminate line.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(ctx, &session, "terminate", &serde_json::json!({}))
}
