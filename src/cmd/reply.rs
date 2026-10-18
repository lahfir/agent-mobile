//! Driver requests and reply rendering; only [`finish`] prints.

use std::time::Duration;

use agent_mobile_core::contract::{Data, Envelope, trim_snapshot};
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use serde_json::Value;

use super::{Session, emit};

pub(crate) fn error_failure(env: &Envelope) -> Failure {
    env.error.as_ref().map_or_else(
        || Failure::local("empty error envelope", "report a bug"),
        Failure::from_error_body,
    )
}

/// A validated wire request, built without session lookup or I/O.
pub struct Request {
    /// Driver route, which can differ from the CLI verb.
    pub verb: &'static str,
    /// Validated JSON payload.
    pub body: Value,
    /// Override for actions that can outlast the default wire budget.
    pub timeout: Option<Duration>,
}

impl Request {
    /// A request using the standard wire budget.
    pub fn new(verb: &'static str, body: Value) -> Self {
        Self {
            verb,
            body,
            timeout: None,
        }
    }

    /// Execute without printing or interpreting the driver's reply.
    pub fn call(&self, session: &Session) -> Result<Envelope, Failure> {
        match self.timeout {
            Some(timeout) => session.call_within(self.verb, &self.body, timeout),
            None => session.call(self.verb, &self.body),
        }
    }
}

/// Render a driver reply as text or a typed failure, without output.
pub fn text(mut env: Envelope, max_depth: Option<u32>) -> Result<String, Failure> {
    if !env.ok {
        return Err(error_failure(&env));
    }
    trim(&mut env, max_depth);
    Ok(format::render(&env).into_owned())
}

/// Apply the shared depth limit before either text or JSON rendering.
pub fn trim(env: &mut Envelope, max_depth: Option<u32>) {
    if let (Some(depth), Some(Data::Snapshot(snap))) = (max_depth, env.data.as_mut())
        && trim_snapshot(snap, depth)
    {
        snap.text = format::tree_lines(&snap.tree);
    }
}

/// ponytail: preserve raw driver JSON only in the CLI printing wrapper.
pub fn finish(mut env: Envelope, json: bool, max_depth: Option<u32>) -> i32 {
    if max_depth.is_some() && !matches!(env.data, Some(Data::Snapshot(_))) && env.ok {
        eprintln!("note: --max-depth only trims snapshot replies");
    }
    if json {
        trim(&mut env, max_depth);
        match serde_json::to_string(&env) {
            Ok(line) => emit(&line),
            Err(error) => {
                eprintln!("error: cannot serialize the reply: {error}");
                return 1;
            }
        }
        return i32::from(!env.ok);
    }
    match text(env, max_depth) {
        Ok(text) => {
            emit(&text);
            0
        }
        Err(failure) => failure.report(),
    }
}

/// `serve --app`: launch on the new session and print the reply.
pub fn round_trip_within(
    session: &Session,
    verb: &str,
    body: &Value,
    timeout: Duration,
    json: bool,
) -> Result<i32, Failure> {
    Ok(finish(
        session.call_within(verb, body, timeout)?,
        json,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::write_line;

    fn fixture(raw: &str) -> Result<Envelope, Failure> {
        Envelope::from_json(raw).map_err(|error| Failure::local(error.to_string(), "fix fixture"))
    }

    #[test]
    fn recorded_snapshot_reply_matches_cli_bytes() -> Result<(), Failure> {
        let env = fixture(include_str!(
            "../../crates/core/tests/fixtures/snapshot-calendar.json"
        ))?;
        let Some(Data::Snapshot(snapshot)) = &env.data else {
            return Err(Failure::local("not a snapshot", "fix fixture"));
        };
        let expected = format!(
            "app={} snapshot=@{} refs={} settled={} reads={} elapsed_ms={}\n{}\n\n",
            snapshot.app,
            snapshot.snapshot_id,
            snapshot.ref_count,
            snapshot.settled,
            snapshot.reads,
            env.elapsed_ms.unwrap_or_default(),
            snapshot.text
        );
        let text = text(env, None)?;
        let mut printed = Vec::new();
        write_line(&mut printed, &text)?;
        assert_eq!(printed, expected.as_bytes());
        assert_eq!(text.as_bytes(), &printed[..printed.len() - 1]);
        Ok(())
    }

    #[test]
    fn stale_ref_is_a_typed_reply_failure() -> Result<(), Failure> {
        let env = fixture(include_str!(
            "../../crates/core/tests/fixtures/error-stale.json"
        ))?;
        let result = text(env, None);
        assert!(matches!(&result, Err(failure) if failure.code_str() == "STALE_REF"));
        Ok(())
    }
}
