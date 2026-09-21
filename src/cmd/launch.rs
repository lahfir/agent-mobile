//! `launch`: cold-start a bundle id; the driver kills saved app state
//! (KTD17 — the skills page teaches `activate` for resumes).

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `launch <bundle_id>`; one round trip returning the settled snapshot.
pub fn run(ctx: &Ctx, bundle_id: &str) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(
        ctx,
        &session,
        "launch",
        &serde_json::json!({ "bundle_id": bundle_id }),
    )
}
