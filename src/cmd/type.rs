//! `type`: a leading ref-shaped positional is consumed as the element to tap
//! first; the rest joins into the text (KTD16). Text that merely starts with
//! `@` is never ref-shaped, so it passes through untouched.

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::{Ctx, round_trip};

/// Run `type`; text is required, the leading ref is not.
pub fn run(ctx: &Ctx, args: &[String]) -> Result<i32, Failure> {
    let (target, text) = split(args)?;
    let mut body = serde_json::json!({ "text": text });
    if let Some(r) = &target {
        body["ref"] = serde_json::json!(r.to_string());
    }
    let session = ctx.session()?;
    if let Some(r) = &target {
        session.check_fresh(r)?;
    }
    round_trip(ctx, &session, "type", &body)
}

fn split(args: &[String]) -> Result<(Option<Ref>, String), Failure> {
    let Some((first, rest)) = args.split_first() else {
        return Err(Failure::usage("type needs text to type"));
    };
    let Ok(r) = Ref::parse(first) else {
        return Ok((None, args.join(" ")));
    };
    if rest.is_empty() {
        return Err(Failure::usage(
            "type needs text after the ref; quote the text or use `--` if it begins with `-`",
        ));
    }
    Ok((Some(r), rest.join(" ")))
}
