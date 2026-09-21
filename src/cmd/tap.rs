//! `tap`: one positional is a ref, two are `x y` points in the app frame
//! (KTD16); the split is validated before any round trip.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `tap` with the one-or-two positional split.
pub fn run(ctx: &Ctx, args: &[String]) -> Result<i32, Failure> {
    let (body, parsed) = match args {
        [r] => (serde_json::json!({ "ref": r }), Some(Ref::parse(r)?)),
        [x, y] => {
            let x = x.parse::<f64>().map_err(|_| {
                Failure::usage(format!(
                    "tap takes a ref or an x y point; {x:?} is not a number"
                ))
            })?;
            let y = y.parse::<f64>().map_err(|_| {
                Failure::usage(format!(
                    "tap takes a ref or an x y point; {y:?} is not a number"
                ))
            })?;
            (serde_json::json!({ "x": x, "y": y }), None)
        }
        _ => {
            return Err(Failure::usage("tap takes a ref or an x y point"));
        }
    };
    let session = ctx.session()?;
    if let Some(r) = &parsed {
        session.check_fresh(r)?;
    }
    round_trip(ctx, &session, "tap", &body)
}
