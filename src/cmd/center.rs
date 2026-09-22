//! `center`: open Notification Center from the `SpringBoard` session. Clap
//! restricts the value to `notification`; anything else exits client-side.

use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `center` with the validated sheet selector.
pub fn run(ctx: &Ctx, which: &str) -> Result<i32, Failure> {
    let session = ctx.session()?;
    round_trip(
        ctx,
        &session,
        "center",
        &serde_json::json!({ "which": which }),
    )
}
