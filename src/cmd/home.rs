//! `home`: go to the launcher/home and return its tree.

use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `home` wire request without session lookup or I/O.
pub fn request() -> Request {
    Request::new("home", serde_json::json!({}))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    request().run(ctx)
}
