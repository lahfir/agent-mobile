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
pub mod reply;
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

use agent_mobile_core::contract::{Envelope, Ref};
use agent_mobile_core::error::Failure;
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
    android: bool,
}

impl Session {
    /// Build a session straight from a URL and token (serve's own calls).
    #[must_use]
    pub fn new(url: String, token: String) -> Self {
        Self {
            url,
            token,
            android: false,
        }
    }

    /// Build a session against the Android bridge: transport failures
    /// render the bridge/`adb` checklist instead of the iOS one.
    #[must_use]
    pub fn new_android(url: String, token: String) -> Self {
        Self {
            url,
            token,
            android: true,
        }
    }

    /// The wire client for this session's platform.
    fn wire(&self, timeout: Option<std::time::Duration>) -> Wire {
        let wire = match timeout {
            Some(t) => Wire::with_timeout(&self.url, &self.token, t),
            None => Wire::new(&self.url, &self.token),
        };
        if self.android {
            wire.for_android()
        } else {
            wire
        }
    }

    /// One driver call with the standard timeout.
    pub fn call(&self, verb: &str, body: &Value) -> Result<Envelope, Failure> {
        self.wire(None).call(verb, body)
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
        self.wire(Some(timeout)).call(verb, body)
    }
}

impl Ctx {
    /// Build the CLI context; `serve` ignores the global device flag.
    fn new(cli: &Cli) -> Result<Self, Failure> {
        let device = if matches!(cli.command, Command::Serve { .. }) {
            None
        } else {
            cli.device.as_deref()
        };
        let mut ctx = Self::from_options(device, cli.app.clone(), cli.max_depth)?;
        ctx.json = cli.json;
        Ok(ctx)
    }

    /// Build a text-reply context without constructing a CLI invocation.
    pub fn from_options(
        device: Option<&str>,
        app: Option<String>,
        max_depth: Option<u32>,
    ) -> Result<Self, Failure> {
        let store = StateStore::new()?;
        let device = device
            .map(|raw| canonical_with_alias(raw, &store, crate::platform::resolve))
            .transpose()?;
        Ok(Self {
            json: false,
            app,
            max_depth,
            device,
            store,
        })
    }

    /// Resolve this invocation's endpoint, or lazy-start a driver first
    /// when nothing is running (KTD7).
    pub fn session(&self) -> Result<Session, Failure> {
        match self.ready_session()? {
            Some(s) => Ok(s),
            None => lazy::session(self),
        }
    }

    /// The session for an already-running driver; `None` on a miss or when
    /// the recorded session's pid is dead — `resolve` reconciles stale
    /// entries to a lazy boot instead of a wire failure. The stale entry
    /// stays on disk for `serve` to reclaim: its `runner_pid` reaps any
    /// orphaned runner still holding the port (KTD7). State keys are
    /// canonical `platform:id` (legacy pre-upgrade rows are iOS display
    /// names, and env-override resolves carry no key), so only an
    /// `android:` key selects the Android transport checklist.
    fn ready_session(&self) -> Result<Option<Session>, Failure> {
        let Some(r) = self.store.resolve(self.device.as_deref())? else {
            return Ok(None);
        };
        let token = r.token().to_owned();
        let android = r
            .device
            .as_deref()
            .is_some_and(|d| d.starts_with("android:"));
        let session = if android {
            Session::new_android(r.url, token)
        } else {
            Session::new(r.url, token)
        };
        Ok(Some(session))
    }
}

/// `raw` -> canonical key, live state aliases before any platform
/// discovery (the injected resolver must not run for a live row).
fn canonical_with_alias(
    raw: &str,
    store: &StateStore,
    discover: impl Fn(&str) -> Result<crate::platform::PlatformDevice, Failure>,
) -> Result<String, Failure> {
    let state = store.load();
    if let Some(key) = state.resolve_live_device_key(raw)? {
        let display = state
            .devices
            .get(&key)
            .and_then(|e| e.device_name.clone())
            .unwrap_or_else(|| raw.to_owned());
        store.remember_device_selection(&key, &display)?;
        return Ok(key);
    }
    let device = discover(raw)?;
    let canonical = device.key();
    store.remember_device_selection(&canonical, device.name())?;
    Ok(canonical)
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
        Command::Mcp => crate::mcp::run(cli),
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
        Command::Mcp => crate::mcp::run(cli),
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
    let mut out = std::io::stdout().lock();
    if write_line(&mut out, line).is_err() {
        std::process::exit(0);
    }
}

/// The CLI's checked write, kept separate from reply rendering.
fn write_line(out: &mut impl std::io::Write, line: &str) -> std::io::Result<()> {
    writeln!(out, "{line}")?;
    out.flush()
}

/// Shared one-or-two positional split for point verbs (`tap`,
/// `doubletap`, `hold`): one argument is a ref, two are finite `x y` numbers, anything
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

pub use reply::round_trip_within;

#[cfg(test)]
mod tests;
