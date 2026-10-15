//! Shared request execution and stdout-clean rendering; only CLI wrappers emit.

use std::time::Duration;

use agent_mobile_core::contract::{Data, Envelope, trim_snapshot};
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use serde_json::Value;

use super::{Ctx, Session, emit};

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

    /// CLI wrapper: resolve the session, execute, then print the reply.
    pub fn run(&self, ctx: &Ctx) -> Result<i32, Failure> {
        let session = ctx.session()?;
        Ok(ctx.finish(self.call(&session)?))
    }
}

impl Ctx {
    /// Render a driver reply as text or a typed failure, without output.
    pub fn reply(&self, mut env: Envelope) -> Result<String, Failure> {
        if !env.ok {
            return Err(env.error.as_ref().map_or_else(
                || Failure::local("empty error envelope", "report a bug"),
                Failure::from_error_body,
            ));
        }
        self.trim_reply(&mut env);
        Ok(format::render(&env).into_owned())
    }

    /// Apply the shared depth limit before either text or JSON rendering.
    fn trim_reply(&self, env: &mut Envelope) {
        if let (Some(depth), Some(Data::Snapshot(snap))) = (self.max_depth, env.data.as_mut())
            && trim_snapshot(snap, depth)
        {
            snap.text = format::tree_lines(&snap.tree);
        }
    }

    /// ponytail: preserve raw driver JSON only in the CLI printing wrapper.
    pub(super) fn finish(&self, mut env: Envelope) -> i32 {
        if self.max_depth.is_some() && !matches!(env.data, Some(Data::Snapshot(_))) && env.ok {
            eprintln!("note: --max-depth only trims snapshot replies");
        }
        if self.json {
            self.trim_reply(&mut env);
            match serde_json::to_string(&env) {
                Ok(line) => emit(&line),
                Err(error) => {
                    eprintln!("error: cannot serialize the reply: {error}");
                    return 1;
                }
            }
            return i32::from(!env.ok);
        }
        match self.reply(env) {
            Ok(text) => {
                emit(&text);
                0
            }
            Err(failure) => failure.report(),
        }
    }
}

/// CLI compatibility wrapper for `serve`'s launch on an existing session.
pub fn round_trip_within(
    ctx: &Ctx,
    session: &Session,
    verb: &str,
    body: &Value,
    timeout: Duration,
) -> Result<i32, Failure> {
    Ok(ctx.finish(session.call_within(verb, body, timeout)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{hold, pinch, write_line};

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
        let ctx = Ctx::from_options(None, None, None)?;
        let text = ctx.reply(env)?;
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
        let ctx = Ctx::from_options(None, None, None)?;
        let result = ctx.reply(env);
        assert!(matches!(&result, Err(failure) if failure.code_str() == "STALE_REF"));
        if std::env::var_os("AM_REPLY_STDOUT_PROBE").is_none() {
            let output = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "cmd::reply::tests::stale_ref_is_a_typed_reply_failure",
                    "--nocapture",
                    "--quiet",
                ])
                .env("AM_REPLY_STDOUT_PROBE", "1")
                .output()?;
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            assert!(
                output.stdout.starts_with(
                    b"\nrunning 1 test\n.\ntest result: ok. 1 passed; 0 failed; 0 ignored;"
                ),
                "reply must not write before the test harness summary: {output:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn non_finite_gestures_fail_before_session_lookup() -> Result<(), Failure> {
        let target = ["@snap1:e2".to_owned()];
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                hold::request(&target, value),
                Err(Failure::Usage { .. })
            ));
            assert!(matches!(
                pinch::request(&target[0], value, None),
                Err(Failure::Usage { .. })
            ));
        }
        let request = pinch::request(&target[0], 1.0, None)?;
        assert_eq!(request.body["scale"], 1.0);
        Ok(())
    }
}
