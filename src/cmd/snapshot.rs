//! `snapshot`: mint refs and return the settled tree; `--app` carries an
//! app selector (iOS targets the bundle; Android asserts the foreground
//! package), and `--max-depth` trims client-side.

use agent_mobile_core::error::Failure;

use super::{Ctx, reply::Request};

/// Build the `snapshot` wire request without session lookup or I/O.
pub fn request(app: Option<&str>) -> Request {
    let body = match app {
        Some(app) => serde_json::json!({ "app": app }),
        None => serde_json::json!({}),
    };
    Request::new("snapshot", body)
}

/// CLI printing wrapper.
pub fn run(ctx: &Ctx) -> Result<i32, Failure> {
    request(ctx.app.as_deref()).run(ctx)
}
