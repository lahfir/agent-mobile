//! `screenshot`: base64 to stdout when no path is given, decoded PNG bytes
//! to a file plus a byte-count confirmation when one is (KTD12).

use agent_mobile_core::contract::Data;
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use base64::Engine as _;

use super::{Call, Ctx, Reply, reply::Request};

/// The `screenshot` verb: base64 in the reply, or PNG bytes written to the
/// CLI-only `output` path with a byte-count line.
///
/// # Errors
/// `--json` with a path, session or transport failures, or a bad payload.
pub fn exec(call: &Call) -> Result<Reply, Failure> {
    let output = call.str("output");
    if call.json && output.is_some() {
        return Err(Failure::usage(
            "`screenshot <path>` writes a file; drop the path or the `--json` flag",
        ));
    }
    let session = Ctx::for_call(call)?.session()?;
    let env = Request::new("screenshot", serde_json::json!({})).call(&session)?;
    match (output, &env.data) {
        (Some(path), Some(Data::Screenshot(shot))) if env.ok => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&shot.png_base64)
                .map_err(|e| {
                    Failure::local(
                        format!("invalid base64 in screenshot payload: {e}"),
                        "report a bug; the driver sent non-base64 bytes",
                    )
                })?;
            std::fs::write(path, &bytes)?;
            Ok(Reply::Text(format::screenshot_written(path, bytes.len())))
        }
        _ => Ok(Reply::Envelope(env)),
    }
}
