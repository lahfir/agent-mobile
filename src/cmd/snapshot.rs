//! `snapshot`: mint refs and return the settled tree; `--app` retargets the
//! bundle, `--max-depth` trims client-side.

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `snapshot`; carries `--app` when given.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    let session = ctx.session()?;
    let body = match &ctx.app {
        Some(app) => serde_json::json!({ "app": app }),
        None => serde_json::json!({}),
    };
    round_trip(ctx, &session, "snapshot", &body)
}
