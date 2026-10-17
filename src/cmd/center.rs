//! `center`: open notifications from the system UI session. Clap restricts
//! the value to `notification`; anything else exits client-side.

use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `center` wire request without session lookup or I/O.
pub fn request(which: &str) -> Request {
    Request::new("center", serde_json::json!({ "which": which }))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx, which: &str) -> Result<i32, Failure> {
    request(which).run(ctx)
}
