//! `twofinger`: two-finger tap on a ref; the ref is required.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `twofinger` wire request without session lookup or I/O.
pub fn request(target: &str) -> Result<Request, Failure> {
    let reference = Ref::parse(target)?;
    Ok(Request::new(
        "twofinger",
        serde_json::json!({ "ref": reference.to_string() }),
    ))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx, target: &str) -> Result<i32, Failure> {
    request(target)?.run(ctx)
}
