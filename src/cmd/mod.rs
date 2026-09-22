//! Command dispatch and the shared plumbing every verb inherits: session
//! resolution and the stdout/stderr + exit-code output contract (KTD5,
//! KTD12).

pub mod back;
pub mod center;
pub mod devices;
pub mod doubletap;
pub mod hold;
pub mod home;
pub mod launch;
pub mod lazy;
pub mod pinch;
pub mod screenshot;
pub mod serve;
pub mod skills;
pub mod snapshot;
pub mod status;
pub mod stop;
pub mod swipe;
pub mod tap;
pub mod twofinger;
pub mod r#type;

use serde_json::Value;

use agent_mobile_core::contract::{Data, Envelope, Ref, trim_snapshot};
use agent_mobile_core::error::Failure;
use agent_mobile_core::format;
use agent_mobile_core::ios;
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
    url: String,
    token: String,
}

impl Session {
    /// Build a session straight from a URL and token (serve's own calls).
    #[must_use]
    pub fn new(url: String, token: String) -> Self {
        Self { url, token }
    }

    /// One driver call with the standard timeout.
    pub fn call(&self, verb: &str, body: &Value) -> Result<Envelope, Failure> {
        Wire::new(&self.url, &self.token).call(verb, body)
    }

    /// One driver call with an explicit timeout — for verbs whose
    /// driver-side work can outrun the default (cold `launch`, long `type`
    /// payloads), so the client does not abandon an action that still runs.
    pub fn call_within(
        &self,
        verb: &str,
        body: &Value,
        timeout: std::time::Duration,
    ) -> Result<Envelope, Failure> {
        Wire::with_timeout(&self.url, &self.token, timeout).call(verb, body)
    }
}

impl Ctx {
    /// Build the shared context; `--device` canonicalizes through
    /// [`canonical_device`].
    fn new(cli: &Cli) -> Result<Self, Failure> {
        let store = StateStore::new()?;
        let device = canonical_device(cli, &store)?;
        Ok(Self {
            json: cli.json,
            app: cli.app.clone(),
            max_depth: cli.max_depth,
            device,
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
    /// the recorded session's pid is dead — `resolve` reconciles stale
    /// entries to a lazy boot instead of a wire failure. The stale entry
    /// stays on disk for `serve` to reclaim: its `runner_pid` reaps any
    /// orphaned runner still holding the port (KTD7).
    fn ready_session(&self) -> Result<Option<Session>, Failure> {
        let Some(r) = self.store.resolve(self.device.as_deref())? else {
            return Ok(None);
        };
        let token = r.endpoint.token().to_owned();
        Ok(Some(Session::new(r.endpoint.url, token)))
    }

    /// Emit one reply: honor `--max-depth`, then write text or JSON to
    /// stdout — driver failures go to stderr with the registry hint.
    fn finish(&self, mut env: Envelope) -> i32 {
        if let (Some(depth), Some(Data::Snapshot(snap))) = (self.max_depth, env.data.as_mut())
            && trim_snapshot(snap, depth)
        {
            snap.text = format::tree_lines(&snap.tree);
        }
        if self.max_depth.is_some() && !matches!(env.data, Some(Data::Snapshot(_))) && env.ok {
            eprintln!("note: --max-depth only trims snapshot replies");
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

/// `--device` -> the canonical name session entries are keyed by: a live
/// session under the raw value wins verbatim (its key is authoritative),
/// otherwise `find_device` maps a name or UDID to the canonical name, and a
/// miss is a usage error so a typo never poisons `default_device` or wedges
/// lazy boot on the wrong state key. `serve` ignores the flag — its device
/// is positional.
fn canonical_device(cli: &Cli, store: &StateStore) -> Result<Option<String>, Failure> {
    let flag_counts = !matches!(cli.command, Command::Serve { .. });
    let Some(raw) = cli.device.as_deref().filter(|_| flag_counts) else {
        return Ok(None);
    };
    let live = store
        .entry(raw)
        .is_some_and(|e| agent_mobile_core::process::pid_alive(e.pid));
    let canonical = if live {
        raw.to_owned()
    } else {
        ios::find_device(raw)?
            .ok_or_else(|| {
                Failure::usage(format!(
                    "no device {raw:?}; `agent-mobile devices` lists reachable devices"
                ))
            })?
            .name
    };
    store.remember_device(&canonical)?;
    Ok(Some(canonical))
}

/// Note when a global flag lands on a verb that ignores it — a flag that
/// silently does nothing is worse than a note on stderr.
fn warn_unused_globals(cli: &Cli, app_ok: bool, device_ok: bool) {
    if cli.app.is_some() && !app_ok {
        eprintln!("note: --app is ignored by {}", cli.command.name());
    }
    if cli.device.is_some() && !device_ok {
        eprintln!("note: --device is ignored by {}", cli.command.name());
    }
}

/// Run `cli` to completion; the process exit code is the return value.
pub fn dispatch(cli: &Cli) -> Result<i32, Failure> {
    match &cli.command {
        Command::Devices => {
            warn_unused_globals(cli, false, false);
            devices::run(cli.json)
        }
        Command::Skills => {
            warn_unused_globals(cli, false, false);
            Ok(skills::run())
        }
        Command::Serve { .. } => {
            warn_unused_globals(cli, true, false);
            ctx_dispatch(cli)
        }
        Command::Snapshot => {
            warn_unused_globals(cli, true, true);
            ctx_dispatch(cli)
        }
        _ => {
            warn_unused_globals(cli, false, true);
            ctx_dispatch(cli)
        }
    }
}

/// Dispatch the verbs that need a session context.
fn ctx_dispatch(cli: &Cli) -> Result<i32, Failure> {
    match &cli.command {
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
        Command::Doubletap { args } => Ctx::new(cli).and_then(|ctx| doubletap::run(&ctx, args)),
        Command::Pinch {
            target,
            scale,
            velocity,
        } => Ctx::new(cli).and_then(|ctx| pinch::run(&ctx, target, *scale, *velocity)),
        Command::Hold { args, duration } => {
            Ctx::new(cli).and_then(|ctx| hold::run(&ctx, args, *duration))
        }
        Command::Back => Ctx::new(cli).and_then(|ctx| back::run(&ctx)),
        Command::Twofinger { target } => Ctx::new(cli).and_then(|ctx| twofinger::run(&ctx, target)),
        Command::Center { which } => Ctx::new(cli).and_then(|ctx| center::run(&ctx, which)),
        Command::Devices => devices::run(cli.json),
        Command::Skills => Ok(skills::run()),
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

/// Shared one-or-two positional split for point verbs (`doubletap`,
/// `hold`): one argument is a ref, two are finite `x y` numbers, anything
/// else is a usage error naming the verb.
pub fn ref_or_point_body(args: &[String], verb: &str) -> Result<Value, Failure> {
    match args {
        [r] => {
            let r = Ref::parse(r)?;
            Ok(serde_json::json!({ "ref": r.to_string() }))
        }
        [x, y] => {
            let point = |raw: &str| {
                raw.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        Failure::usage(format!(
                            "{verb} takes a ref or an x y point; {raw:?} is not a finite number"
                        ))
                    })
            };
            let (x, y) = (point(x)?, point(y)?);
            Ok(serde_json::json!({ "x": x, "y": y }))
        }
        _ => Err(Failure::usage(format!(
            "{verb} takes a ref or an x y point"
        ))),
    }
}

/// One wire round trip plus [`Ctx::finish`]; the shape every thin verb shares.
pub fn round_trip(ctx: &Ctx, session: &Session, verb: &str, body: &Value) -> Result<i32, Failure> {
    let env = session.call(verb, body)?;
    Ok(ctx.finish(env))
}

/// `round_trip` with a longer wire timeout for verbs whose driver-side work
/// can legitimately outrun the default — abandoning mid-action is worse
/// than waiting.
pub fn round_trip_within(
    ctx: &Ctx,
    session: &Session,
    verb: &str,
    body: &Value,
    timeout: std::time::Duration,
) -> Result<i32, Failure> {
    let env = session.call_within(verb, body, timeout)?;
    Ok(ctx.finish(env))
}
