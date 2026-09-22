//! `swipe`: clap's `value_parser` rejects bad directions client-side; an
//! optional trailing ref scopes the swipe to one element.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `swipe` with an optional ref target.
pub fn run(ctx: &Ctx, direction: &str, target: Option<&str>) -> Result<i32, Failure> {
    let mut body = serde_json::json!({ "direction": direction });
    if let Some(r) = target {
        body["ref"] = serde_json::json!(Ref::parse(r)?.to_string());
    }
    let session = ctx.session()?;
    round_trip(ctx, &session, "swipe", &body)
}
