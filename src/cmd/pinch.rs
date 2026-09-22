//! `pinch`: zoom on a ref with a scale factor. Clap owns the `f64` parse,
//! so near-1 scales ride the wire for the driver to reject; non-finite
//! scales never reach the wire (see `run`).

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `pinch`; scale bands are the driver's call, never the CLI's.
/// Non-finite scales and velocities are rejected here, mirroring `hold`:
/// JSON cannot carry them, so `serde_json` would silently encode null and
/// the driver would reject a body that no longer says what the user typed.
pub fn run(ctx: &Ctx, target: &str, scale: f64, velocity: Option<f64>) -> Result<i32, Failure> {
    if !scale.is_finite() {
        return Err(Failure::usage("pinch scale must be a finite number"));
    }
    if velocity.is_some_and(|v| !v.is_finite()) {
        return Err(Failure::usage("pinch velocity must be a finite number"));
    }
    let r = Ref::parse(target)?;
    let mut body = serde_json::json!({ "ref": r.to_string(), "scale": scale });
    if let Some(v) = velocity {
        body["velocity"] = serde_json::json!(v);
    }
    let session = ctx.session()?;
    round_trip(ctx, &session, "pinch", &body)
}
