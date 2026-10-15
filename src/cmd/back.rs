//! `back`: perform the system edge swipe back; no target, empty body.

use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `back` wire request without session lookup or I/O.
pub fn request() -> Request {
    Request::new("back", serde_json::json!({}))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    request().run(ctx)
}
