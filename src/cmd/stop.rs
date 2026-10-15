//! `stop`: sends `terminate` — the app dies, the driver stays up, and the
//! driver's bundle resets to springboard (KTD10).

use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `stop` wire request without session lookup or I/O.
pub fn request() -> Request {
    Request::new("terminate", serde_json::json!({}))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    request().run(ctx)
}
