//! `twofinger`: two-finger tap on a ref; the ref is required.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `twofinger` on one validated ref.
pub fn run(ctx: &Ctx, target: &str) -> Result<i32, Failure> {
    let r = Ref::parse(target)?;
    let session = ctx.session()?;
    round_trip(
        ctx,
        &session,
        "twofinger",
        &serde_json::json!({ "ref": r.to_string() }),
    )
}
