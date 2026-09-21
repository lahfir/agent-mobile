//! `swipe`: a direction enum rejects bad values client-side; an optional
//! trailing ref scopes the swipe to one element.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use crate::cli::Direction;

use super::{Ctx, round_trip};

/// Run `swipe` with an optional ref target.
pub fn run(ctx: &Ctx, direction: Direction, target: Option<&str>) -> Result<i32, Failure> {
    let parsed = target.map(Ref::parse).transpose()?;
    let mut body = serde_json::json!({ "direction": direction.as_str() });
    if let Some(r) = target {
        body["ref"] = serde_json::json!(r);
    }
    let session = ctx.session()?;
    if let Some(r) = &parsed {
        session.check_fresh(r)?;
    }
    round_trip(ctx, &session, "swipe", &body)
}
