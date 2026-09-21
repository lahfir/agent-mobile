//! Command dispatch and the shared plumbing every verb inherits: session
//! resolution and the stdout/stderr + exit-code output contract (KTD5,
//! KTD12).

pub mod devices;
pub mod home;
pub mod launch;
pub mod lazy;
pub mod screenshot;
pub mod serve;
pub mod skills;
pub mod snapshot;
pub mod status;
pub mod stop;
pub mod swipe;
pub mod tap;
pub mod r#type;

use serde_json::Value;

use agent_mobile_core::contract::{Data, Envelope, trim_snapshot};
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use agent_mobile_core::state::StateStore;
use agent_mobile_core::wire::Wire;

use crate::cli::{Cli, Command};

/// Parsed global flags plus the session store, shared by every verb.
pub struct Ctx {
    json: bool,
    app: Option<String>,
    max_depth: Option<u32>,
    device: Option<String>,
    store: StateStore,
}

/// A resolved driver endpoint. Ref freshness is the driver's job — it holds
/// the live tree and answers `STALE_REF` itself; the CLI stays stateless.
pub struct Session {
    wire: Wire,
}

impl Ctx {
    /// Build the shared context, honoring `--device` by remembering it.
    fn new(cli: &Cli) -> Result<Self, Failure> {
        let store = StateStore::new()?;
        if let Some(name) = &cli.device {
            store.remember_device(name)?;
        }
        Ok(Self {
            json: cli.json,
            app: cli.app.clone(),
            max_depth: cli.max_depth,
            device: cli.device.clone(),
            store,
        })
    }

    /// Resolve this invocation's endpoint, or lazy-start a driver first
    /// when nothing is running (KTD7).
    fn session(&self) -> Result<Session, Failure> {
        match self.ready_session()? {
            Some(s) => Ok(s),
            None => lazy::session(self),
        }
    }

    /// The session for an already-running driver; `None` on a miss or when
    /// the recorded session's pid is dead — stale state reconciles to a
    /// lazy boot instead of a wire failure. The stale entry stays on disk
    /// for `serve` to reclaim: its `runner_pid` reaps any orphaned runner
    /// still holding the port (KTD7).
    fn ready_session(&self) -> Result<Option<Session>, Failure> {
        let Some(r) = self.store.resolve(self.device.as_deref())? else {
            return Ok(None);
        };
        if let Some(e) = &r.entry
            && !agent_mobile_core::process::pid_alive(e.pid)
        {
            return Ok(None);
        }
        Ok(Some(Session {
            wire: Wire::new(&r.endpoint.url, r.endpoint.token()),
        }))
    }

    /// Emit one reply: honor `--max-depth`, then write text or JSON to
    /// stdout — driver failures go to stderr with the registry hint.
    fn finish(&self, mut env: Envelope) -> i32 {
        if let (Some(depth), Some(Data::Snapshot(snap))) = (self.max_depth, env.data.as_mut())
            && trim_snapshot(snap, depth)
        {
            snap.text = format::tree_lines(&snap.tree);
        }
        if self.json {
            match serde_json::to_string(&env) {
                Ok(line) => emit(&line),
                Err(e) => {
                    eprintln!("error: cannot serialize the reply: {e}");
                    return 1;
                }
            }
            return i32::from(!env.ok);
        }
        if env.ok {
            emit(&format::render(&env));
            0
        } else {
            env.error
                .as_ref()
                .map_or_else(
                    || Failure::local("empty error envelope", "report a bug"),
                    Failure::from_error_body,
                )
                .report()
        }
    }
}

/// Run `cli` to completion; the process exit code is the return value.
pub fn dispatch(cli: &Cli) -> Result<i32, Failure> {
    match &cli.command {
        Command::Devices => devices::run(cli.json),
        Command::Skills => Ok(skills::run()),
        Command::Status => Ctx::new(cli).and_then(|ctx| status::run(&ctx)),
        Command::Snapshot => Ctx::new(cli).and_then(|ctx| snapshot::run(&ctx)),
        Command::Tap { args } => Ctx::new(cli).and_then(|ctx| tap::run(&ctx, args)),
        Command::Type { args } => Ctx::new(cli).and_then(|ctx| r#type::run(&ctx, args)),
        Command::Swipe { direction, target } => {
            Ctx::new(cli).and_then(|ctx| swipe::run(&ctx, direction, target.as_deref()))
        }
        Command::Home => Ctx::new(cli).and_then(|ctx| home::run(&ctx)),
        Command::Launch { bundle_id } => Ctx::new(cli).and_then(|ctx| launch::run(&ctx, bundle_id)),
        Command::Screenshot { output } => {
            Ctx::new(cli).and_then(|ctx| screenshot::run(&ctx, output.as_deref()))
        }
        Command::Stop => Ctx::new(cli).and_then(|ctx| stop::run(&ctx)),
        Command::Serve { device } => Ctx::new(cli).and_then(|ctx| serve::run(&ctx, device)),
    }
}

/// Create the `~/.agent-mobile` state dir before locks or logs touch it.
pub fn ensure_state_dir(store: &StateStore) -> Result<(), Failure> {
    agent_mobile_core::secret::create_private_dirs(store.root())
}

/// Write one line to stdout; a closed pipe (`| head`) is a clean exit, not
/// a panic (KTD8). `println!` panics on EPIPE, so every stdout write routes
/// here.
pub fn emit(line: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{line}").is_err() || out.flush().is_err() {
        std::process::exit(0);
    }
}

/// One wire round trip plus [`Ctx::finish`]; the shape every thin verb shares.
pub fn round_trip(ctx: &Ctx, session: &Session, verb: &str, body: &Value) -> Result<i32, Failure> {
    let env = session.wire.call(verb, body)?;
    Ok(ctx.finish(env))
}
