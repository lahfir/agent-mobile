//! `activate`: foreground a running app without relaunch — the resume that
//! preserves state, per KTD9's launch/activate split.

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `activate <bundle_id>`; one round trip returning the settled snapshot.
pub fn run(ctx: &Ctx, bundle_id: &str) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(
        ctx,
        &session,
        "activate",
        &serde_json::json!({ "bundle_id": bundle_id }),
    )
}
