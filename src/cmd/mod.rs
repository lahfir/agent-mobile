//! Command dispatch and the shared plumbing every verb inherits: session
//! resolution, local ref checks, snapshot bookkeeping, and the
//! stdout/stderr + exit-code output contract (KTD5, KTD12).

pub mod activate;
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

use agent_mobile_core::contract::{Data, Envelope, Ref, trim_snapshot};
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use agent_mobile_core::state::{ResolveOutcome, StateStore, URL_ENV};
use agent_mobile_core::wire::{Reply, Wire};

use crate::cli::{Cli, Command};

/// Parsed global flags plus the session store, shared by every verb.
pub struct Ctx {
    json: bool,
    app: Option<String>,
    max_depth: Option<u32>,
    device: Option<String>,
    store: StateStore,
}

/// A resolved driver endpoint plus the bookkeeping needed to validate refs
/// locally and record fresh snapshot ids.
pub struct Session {
    wire: Wire,
    device: Option<String>,
    last_snapshot_id: Option<String>,
}

impl Session {
    /// Reject a ref minted under a different snapshot than this session's
    /// latest — locally, before any round trip. No recorded id means the
    /// driver alone decides.
    fn check_fresh(&self, r: &Ref) -> Result<(), Failure> {
        match &self.last_snapshot_id {
            Some(id) => r.check_current(id),
            None => Ok(()),
        }
    }
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
        let ResolveOutcome::Ready(ep) = self.store.resolve(self.device.as_deref())? else {
            return Ok(None);
        };
        let env_pinned = std::env::var(URL_ENV).is_ok();
        let entry = if env_pinned {
            None
        } else {
            ep.device.as_deref().and_then(|d| self.store.entry(d))
        };
        if let Some(e) = &entry
            && !agent_mobile_core::process::pid_alive(e.pid)
        {
            return Ok(None);
        }
        Ok(Some(Session {
            wire: Wire::new(&ep.url, ep.token()),
            device: ep.device.clone(),
            last_snapshot_id: entry.and_then(|e| e.last_snapshot_id),
        }))
    }

    /// Emit one reply: record the snapshot id, honor `--max-depth`, then
    /// write text or JSON to stdout — driver failures go to stderr with the
    /// registry hint.
    fn finish(&self, session: &Session, mut env: Envelope) -> i32 {
        if let (Some(depth), Some(Data::Snapshot(snap))) = (self.max_depth, env.data.as_mut()) {
            trim_snapshot(snap, depth);
        }
        self.record_snapshot(session, &env);
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
            let f = env.error.as_ref().map_or_else(
                || Failure::local("empty error envelope", "report a bug"),
                Failure::from_error_body,
            );
            eprintln!("{}", f.render());
            f.exit_code()
        }
    }

    /// Persist the driver's newest snapshot id for the resolved device;
    /// failures warn on stderr but never sink a successful verb.
    fn record_snapshot(&self, session: &Session, env: &Envelope) {
        let (Some(device), Some(Data::Snapshot(snap)), true) = (&session.device, &env.data, env.ok)
        else {
            return;
        };
        if let Err(e) = self.store.record_snapshot(device, &snap.snapshot_id) {
            eprintln!("warning: {}", e.render());
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
            Ctx::new(cli).and_then(|ctx| swipe::run(&ctx, *direction, target.as_deref()))
        }
        Command::Home => Ctx::new(cli).and_then(|ctx| home::run(&ctx)),
        Command::Launch { bundle_id } => Ctx::new(cli).and_then(|ctx| launch::run(&ctx, bundle_id)),
        Command::Activate { bundle_id } => {
            Ctx::new(cli).and_then(|ctx| activate::run(&ctx, bundle_id))
        }
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
    let reply: Reply = session.wire.call(verb, body)?;
    Ok(ctx.finish(session, reply.envelope))
}
