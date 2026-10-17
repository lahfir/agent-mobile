//! `status`: the driver's identity reply — app, device, os, snapshot id.

use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `status` wire request without session lookup or I/O.
pub fn request() -> Request {
    Request::new("status", serde_json::json!({}))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    request().run(ctx)
}
