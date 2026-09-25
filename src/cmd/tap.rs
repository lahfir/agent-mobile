//! `tap`: one positional is a ref, two are `x y` points in the app frame
//! (KTD16); the split is validated before any round trip.

use agent_mobile_core::error::Failure;

use super::{Ctx, ref_or_point_body, round_trip};

/// Run `tap` with the one-or-two positional split.
pub fn run(ctx: &Ctx, args: &[String]) -> Result<i32, Failure> {
    let body = ref_or_point_body(args, "tap")?;
    let session = ctx.session()?;
    round_trip(ctx, &session, "tap", &body)
}
