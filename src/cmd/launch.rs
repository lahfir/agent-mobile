//! `launch`: cold-start a bundle id; the driver kills saved app state
//! (KTD17).

use agent_mobile_core::error::Failure;
use agent_mobile_core::wire::LONG_TIMEOUT;

use super::{Ctx, reply::Request};

/// Build the `launch` wire request without session lookup or I/O.
/// A cold app start can outrun the default wire timeout.
pub fn request(bundle_id: &str) -> Request {
    Request {
        verb: "launch",
        body: serde_json::json!({ "bundle_id": bundle_id }),
        timeout: Some(LONG_TIMEOUT),
    }
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx, bundle_id: &str) -> Result<i32, Failure> {
    request(bundle_id).run(ctx)
}
