//! `screenshot`: base64 to stdout when no path is given, decoded PNG bytes
//! to a file plus a byte-count confirmation when one is (KTD12).

use agent_mobile_core::contract::Data;
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use base64::Engine as _;

use super::Ctx;

/// Run `screenshot [output-path]`.
pub fn run(ctx: &Ctx, output: Option<&str>) -> Result<i32, Failure> {
    if ctx.json && output.is_some() {
        return Err(Failure::usage(
            "`screenshot <path>` writes a file; drop the path or the `--json` flag",
        ));
    }
    let session = ctx.session()?;
    let env = session.call("screenshot", &serde_json::json!({}))?;
    let path = if ctx.json { None } else { output };
    let Some(path) = path else {
        return Ok(ctx.finish(env));
    };
    let Some(Data::Screenshot(shot)) = &env.data else {
        return Ok(ctx.finish(env));
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
    super::emit(&format::screenshot_written(path, bytes.len()));
    Ok(0)
}
