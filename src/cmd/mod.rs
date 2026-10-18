//! Execution shared by both surfaces: a validated [`Call`] runs through
//! [`execute`] to one [`Reply`] (KTD5); session resolution and the
//! stdout/stderr + exit-code contract live here (KTD12).

pub mod devices;
pub mod lazy;
pub mod release;
pub mod reply;
pub mod screenshot;
pub mod scroll;
pub mod serve;
pub mod skills;

use serde_json::Value;

use agent_mobile_core::contract::Envelope;
use agent_mobile_core::error::Failure;
use agent_mobile_core::state::StateStore;
use agent_mobile_core::wire::Wire;

use crate::surface::{Args, Exec, Verb, wire_request};

/// A validated invocation from either surface.
pub struct Call {
    /// The verb being run.
    pub verb: &'static Verb,
    /// Validated arguments, globals included.
    pub args: Args,
    /// CLI `--json`.
    pub json: bool,
    /// Whether device resolution may change the remembered default (KTD8).
    pub remember: bool,
}

impl Call {
    /// A string argument.
    #[must_use]
    pub fn str(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(Value::as_str)
    }

    /// The depth limit, when given.
    #[must_use]
    pub fn max_depth(&self) -> Option<u32> {
        self.args
            .get("max_depth")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    }
}

/// What a verb produced; each surface renders it (KTD5).
pub enum Reply {
    /// A driver reply.
    Envelope(Envelope),
    /// Device rows, plus their text form.
    Devices {
        /// JSON rows.
        rows: Vec<Value>,
        /// One text line per row.
        text: String,
    },
    /// Plain text.
    Text(String),
    /// A CLI-only verb that printed its own output and exits with this code.
    Exit(i32),
}

/// Run a call to its reply.
///
/// # Errors
/// Session, transport, or local failures.
pub fn execute(call: &Call) -> Result<Reply, Failure> {
    match call.verb.exec {
        Exec::Local(run) => run(call),
        Exec::Wire { .. } => {
            let request = wire_request(call.verb, &call.args)
                .ok_or_else(|| Failure::local("not a wire verb", "report a bug"))?;
            let session = Ctx::for_call(call)?.session()?;
            request.call(&session).map(Reply::Envelope)
        }
    }
}

/// Run a CLI call and print its reply; the return value is the exit code.
///
/// # Errors
/// Any failure from [`execute`].
pub fn run_cli(call: &Call) -> Result<i32, Failure> {
    Ok(match execute(call)? {
        Reply::Envelope(env) => reply::finish(env, call.json, call.max_depth()),
        Reply::Devices { rows, text } => {
            if call.json {
                emit(&serde_json::json!({ "devices": rows }).to_string());
            } else if !text.is_empty() {
                emit(&text);
            }
            0
        }
        Reply::Text(text) => {
            emit(&text);
            0
        }
        Reply::Exit(code) => code,
    })
}

/// The `mcp` verb: serve until stdin closes.
///
/// # Errors
/// Only a stdout failure other than a closed pipe.
pub fn mcp_exec(call: &Call) -> Result<Reply, Failure> {
    crate::mcp::run(call).map(Reply::Exit)
}

/// Parsed selectors plus the session store, for verbs that talk to a driver.
pub struct Ctx {
    app: Option<String>,
    device: Option<String>,
    remember: bool,
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
    ///
    /// # Errors
    /// Transport or protocol failures.
    pub fn call(&self, verb: &str, body: &Value) -> Result<Envelope, Failure> {
        self.wire(None).call(verb, body)
    }

    /// One driver call with an explicit timeout — for verbs whose
    /// driver-side work can outrun the default (cold `launch`, long `type`
    /// payloads), so the client does not abandon an action that still runs.
    ///
    /// # Errors
    /// Transport or protocol failures.
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
    /// The context for `call`'s device and app selectors.
    ///
    /// # Errors
    /// An unwritable state dir or an unknown device.
    pub fn for_call(call: &Call) -> Result<Self, Failure> {
        Self::from_options(call.str("device"), call.str("app"), call.remember)
    }

    /// Build a context; `remember` lets device resolution update the
    /// remembered default (the CLI does, MCP does not — KTD8).
    ///
    /// # Errors
    /// An unwritable state dir or an unknown device.
    pub fn from_options(
        device: Option<&str>,
        app: Option<&str>,
        remember: bool,
    ) -> Result<Self, Failure> {
        let store = StateStore::new()?;
        let device = device
            .map(|raw| canonical_with_alias(raw, &store, crate::platform::resolve, remember))
            .transpose()?;
        Ok(Self {
            app: app.map(str::to_owned),
            device,
            remember,
            store,
        })
    }

    /// The app selector, for multi-call host-side verbs.
    #[must_use]
    pub fn app_selector(&self) -> Option<&str> {
        self.app.as_deref()
    }

    /// Resolve this invocation's endpoint, or lazy-start a driver first
    /// when nothing is running (KTD7).
    ///
    /// # Errors
    /// Boot or state failures.
    pub fn session(&self) -> Result<Session, Failure> {
        match self.ready_session(self.device.as_deref())? {
            Some(s) => Ok(s),
            None => lazy::session(self),
        }
    }

    /// The session for an already-running driver for `device` (or the
    /// default); `None` on a miss or when the recorded pid is dead, so
    /// stale entries reconcile to a lazy boot instead of a wire failure.
    /// Only an `android:` key selects the Android transport checklist.
    fn ready_session(&self, device: Option<&str>) -> Result<Option<Session>, Failure> {
        let Some(r) = self.store.resolve(device)? else {
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
    remember: bool,
) -> Result<String, Failure> {
    let state = store.load();
    if let Some(key) = state.resolve_live_device_key(raw)? {
        if remember {
            let display = state
                .devices
                .get(&key)
                .and_then(|e| e.device_name.clone())
                .unwrap_or_else(|| raw.to_owned());
            store.remember_device_selection(&key, &display)?;
        }
        return Ok(key);
    }
    served_elsewhere(&state, None)?;
    let device = discover(raw)?;
    let canonical = device.key();
    if remember {
        store.remember_device_selection(&canonical, device.name())?;
    }
    Ok(canonical)
}

/// One driver runs at a time (R9): a live serve for any device other than
/// `except` fails at once, naming it and pointing to `release`.
fn served_elsewhere(
    state: &agent_mobile_core::state::State,
    except: Option<&str>,
) -> Result<(), Failure> {
    let live = state.devices.iter().find(|(key, entry)| {
        except != Some(key.as_str())
            && agent_mobile_core::process::process_matches(
                entry.pid,
                entry.process_started_at.as_deref(),
            )
    });
    match live {
        Some((key, entry)) => Err(Failure::local(
            format!(
                "{key} is being served (pid {}); one driver runs at a time",
                entry.pid
            ),
            "run `agent-mobile release` (MCP: call the `release` tool), then retry",
        )),
        None => Ok(()),
    }
}

/// Create the `~/.agent-mobile` state dir before locks or logs touch it.
///
/// # Errors
/// An unwritable home.
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

fn write_line(out: &mut impl std::io::Write, line: &str) -> std::io::Result<()> {
    writeln!(out, "{line}")?;
    out.flush()
}

#[cfg(test)]
mod tests;
