//! `launch`: cold-start a bundle id; the driver kills saved app state
//! (KTD17).

use agent_mobile_core::error::Failure;
use agent_mobile_core::wire::LONG_TIMEOUT;

use super::{Ctx, round_trip_within};

/// Run `launch <bundle_id>`; one round trip returning the settled snapshot.
/// A cold app start can outrun the default wire timeout.
pub fn run(ctx: &Ctx, bundle_id: &str) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip_within(
        ctx,
        &session,
        "launch",
        &serde_json::json!({ "bundle_id": bundle_id }),
        LONG_TIMEOUT,
    )
}
