//! `screenshot`: base64 to stdout when no path is given, decoded PNG bytes
//! to a file plus a byte-count confirmation when one is (KTD12).

use agent_mobile_core::contract::{Data, Envelope};
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use base64::Engine as _;

use super::{Ctx, reply::Request};

/// The screenshot wire request, independent of output destination.
pub fn request() -> Request {
    Request::new("screenshot", serde_json::json!({}))
}

/// Run `screenshot [output-path]`.
pub fn run(ctx: &Ctx, output: Option<&str>) -> Result<i32, Failure> {
    if ctx.json && output.is_some() {
        return Err(Failure::usage(
            "`screenshot <path>` writes a file; drop the path or the `--json` flag",
        ));
    }
    let session = ctx.session()?;
    let env = request().call(&session)?;
    let path = if ctx.json { None } else { output };
    let Some(path) = path else {
        return Ok(ctx.finish(env));
    };
    if !matches!(env.data, Some(Data::Screenshot(_))) {
        return Ok(ctx.finish(env));
    }
    super::emit(&reply(ctx, env, Some(path))?);
    Ok(0)
}

/// Return screenshot text, optionally writing PNG bytes, without printing.
pub fn reply(ctx: &Ctx, env: Envelope, output: Option<&str>) -> Result<String, Failure> {
    let (Some(path), Some(Data::Screenshot(shot))) = (output, &env.data) else {
        return ctx.reply(env);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&shot.png_base64)
        .map_err(|e| {
            Failure::local(
                format!("invalid base64 in screenshot payload: {e}"),
                "report a bug; the driver sent non-base64 bytes",
            )
        })?;
    std::fs::write(path, &bytes)?;
    Ok(format::screenshot_written(path, bytes.len()))
}
