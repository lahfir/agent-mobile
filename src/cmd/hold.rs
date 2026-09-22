//! `hold`: press a ref or an `x y` point for a duration in seconds. The
//! target split mirrors `tap`; duration defaults to 1.0 on the CLI while the
//! driver owns positivity validation.

use agent_mobile_core::error::Failure;

use super::{Ctx, ref_or_point_body, round_trip_within};

/// Run `hold` with the one-or-two positional split plus a duration.
/// Non-finite durations are rejected here: JSON cannot carry them, so
/// `serde_json` would silently encode null and the driver would fall back
/// to the default. The wire budget scales with duration (press + settle +
/// headroom) because a fixed 30 s ceiling abandons long presses mid-touch
/// while the driver keeps holding.
pub fn run(ctx: &Ctx, args: &[String], duration: f64) -> Result<i32, Failure> {
    if !duration.is_finite() {
        return Err(Failure::usage(
            "hold duration must be a finite number of seconds",
        ));
    }
    let mut body = ref_or_point_body(args, "hold")?;
    body["duration"] = serde_json::json!(duration);
    let session = ctx.session()?;
    // Clamped so a huge --duration cannot wedge the client; the driver
    // caps accepted holds far below this either way.
    let wait = duration.clamp(0.0, 60.0);
    let budget = std::time::Duration::from_secs(30) + std::time::Duration::from_secs_f64(wait);
    round_trip_within(ctx, &session, "hold", &body, budget)
}
