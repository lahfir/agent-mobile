//! `tap`: one positional is a ref, two are `x y` points in the app frame
//! (KTD16); the split is validated before any round trip.

use agent_mobile_core::error::Failure;

use super::{Ctx, ref_or_point_body, reply::Request};

/// Build the `tap` wire request without session lookup or I/O.
pub fn request(args: &[String]) -> Result<Request, Failure> {
    let body = ref_or_point_body(args, "tap")?;
    Ok(Request::new("tap", body))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx, args: &[String]) -> Result<i32, Failure> {
    request(args)?.run(ctx)
}
