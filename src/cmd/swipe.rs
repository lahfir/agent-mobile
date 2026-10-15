//! `swipe`: clap's `value_parser` rejects bad directions client-side; an
//! optional trailing ref scopes the swipe to one element.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `swipe` wire request without session lookup or I/O.
pub fn request(direction: &str, target: Option<&str>) -> Result<Request, Failure> {
    let mut body = serde_json::json!({ "direction": direction });
    if let Some(r) = target {
        body["ref"] = serde_json::json!(Ref::parse(r)?.to_string());
    }
    Ok(Request::new("swipe", body))
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx, direction: &str, target: Option<&str>) -> Result<i32, Failure> {
    request(direction, target)?.run(ctx)
}
