//! `screenshot`: base64 to stdout when no path is given, decoded PNG bytes
//! to a file plus a byte-count confirmation when one is (KTD12).

use agent_mobile_core::contract::Data;
use agent_mobile_core::error::Failure;
use agent_mobile_core::{b64, format};

use super::Ctx;

/// Run `screenshot [output-path]`.
pub fn run(ctx: &Ctx, output: Option<&str>) -> Result<i32, Failure> {
    let session = ctx.session()?;
    let env = session.wire.call("screenshot", &serde_json::json!({}))?;
    let path = if ctx.json { None } else { output };
    let Some(path) = path else {
        return Ok(ctx.finish(env));
    };
    let Some(Data::Screenshot(shot)) = &env.data else {
        return Ok(ctx.finish(env));
    };
    let bytes = b64::decode(&shot.png_base64)?;
    std::fs::write(path, &bytes)?;
    super::emit(&format::screenshot_written(path, bytes.len()));
    Ok(0)
}
