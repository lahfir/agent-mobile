//! Session state under `~/.agent-mobile` (KTD6, KTD7): which driver serves
//! which device, where each session's token file lives, and the env overrides
//! that win per invocation. A corrupt state file counts as stale, never fatal.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Failure;
use crate::secret::{create_private_dirs, token_file_name, validate_token_file_name, write_secret};

/// State schema version; anything else loads as stale.
const STATE_VERSION: u32 = 1;
/// Basename of the state file inside the state dir.
const STATE_FILE: &str = "state.json";
/// Subdir holding per-device token files.
const TOKENS_DIR: &str = "tokens";
/// Env var overriding the driver URL for one invocation.
pub const URL_ENV: &str = "AGENT_MOBILE_URL";
/// Env var overriding the driver token for one invocation.
pub const TOKEN_ENV: &str = "AGENT_MOBILE_TOKEN";

/// One device's live session: where the driver listens, which process owns
/// it, and which token file holds the bearer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Base URL of the driver, e.g. `http://127.0.0.1:8770`.
    pub url: String,
    /// Pid of the process that owns the session.
    pub pid: u32,
    /// Token file name inside `tokens/`; the token never appears here.
    pub token_file: String,
    /// Pid of the `xcodebuild` runner `serve` spawned — the owned child to
    /// reap when the serve pid dies without cleanup (KTD17).
    #[serde(default)]
    pub runner_pid: Option<u32>,
}

impl SessionEntry {
    /// Record a session.
    #[must_use]
    pub fn new(url: String, pid: u32, token_file: String) -> Self {
        Self {
            url,
            pid,
            token_file,
            runner_pid: None,
        }
    }
}

/// On-disk state: the remembered default device plus one entry per live
/// driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Schema version; must equal [`STATE_VERSION`] to load.
    pub version: u32,
    /// Device name `--device` last selected, reused when omitted.
    #[serde(default)]
    pub default_device: Option<String>,
    /// Live sessions keyed by device name.
    #[serde(default)]
    pub devices: BTreeMap<String, SessionEntry>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            default_device: None,
            devices: BTreeMap::new(),
        }
    }
}

/// The endpoint one invocation should talk to: URL plus bearer token.
/// `Debug` redacts the token.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedEndpoint {
    /// Driver base URL.
    pub url: String,
    /// Device name the endpoint resolved through, when one was selected.
    pub device: Option<String>,
    token: String,
}

impl ResolvedEndpoint {
    /// The bearer token for the `Authorization` header.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl std::fmt::Debug for ResolvedEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedEndpoint")
            .field("url", &self.url)
            .field("device", &self.device)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// File-backed session store rooted at `~/.agent-mobile` (or an injected dir
/// in tests). The root is created on first write, not on construction.
pub struct StateStore {
    root: PathBuf,
}

impl StateStore {
    /// Store at the default `~/.agent-mobile` location.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when `$HOME` is not set.
    pub fn new() -> Result<Self, Failure> {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            Failure::local("cannot locate the home directory", "set HOME and retry")
        })?;
        Ok(Self::at(&PathBuf::from(home).join(".agent-mobile")))
    }

    /// Store rooted at `root`; tests inject a temp dir.
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// The state directory root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of the state file.
    #[must_use]
    pub fn state_file(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    /// Path of the lazy-boot lockfile, taken with `create_new` (KTD7).
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.root.join("boot.lock")
    }

    /// Path of the serve single-instance lockfile, held with `File::lock`.
    #[must_use]
    pub fn serve_lock_file(&self) -> PathBuf {
        self.root.join("serve.lock")
    }

    /// Path of the state mutation lockfile, held with `File::lock` around
    /// every load-mutate-save cycle.
    #[must_use]
    pub fn state_lock_file(&self) -> PathBuf {
        self.root.join("state.lock")
    }

    /// Path of the driver log for `device` (`serve` redirects the runner's
    /// output here; failures stay visible per KTD8).
    #[must_use]
    pub fn driver_log(&self, device: &str) -> PathBuf {
        self.root
            .join(format!("driver-{}.log", Self::token_file_for(device)))
    }

    /// Path of a named token file inside `tokens/`.
    #[must_use]
    pub fn token_path(&self, token_file: &str) -> PathBuf {
        self.root.join(TOKENS_DIR).join(token_file)
    }

    /// Load the state file; missing or corrupt loads as empty (stale).
    #[must_use]
    pub fn load(&self) -> State {
        let Ok(raw) = fs::read_to_string(self.state_file()) else {
            return State::default();
        };
        let Ok(state) = serde_json::from_str::<State>(&raw) else {
            return State::default();
        };
        if state.version == STATE_VERSION {
            state
        } else {
            State::default()
        }
    }

    /// Persist the state atomically (temp file plus rename) inside the
    /// `0o700` state dir.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the dir cannot be created or the write
    /// fails.
    pub fn save(&self, state: &State) -> Result<(), Failure> {
        create_private_dirs(&self.root)?;
        let body = serde_json::to_string_pretty(state)
            .map_err(|e| Failure::local(format!("cannot serialize state: {e}"), "report a bug"))?;
        let tmp = self
            .root
            .join(format!(".state.json.tmp.{}", std::process::id()));
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        {
            let mut file = opts.open(&tmp)?;
            file.write_all(body.as_bytes())?;
        }
        fs::rename(&tmp, self.state_file())?;
        Ok(())
    }

    /// The entry for `device`, if any.
    #[must_use]
    pub fn entry(&self, device: &str) -> Option<SessionEntry> {
        self.load().devices.get(device).cloned()
    }

    /// Run one load-mutate-save cycle under the `state.lock` flock so
    /// concurrent verbs cannot last-writer-wins drop each other's entries.
    /// `f` returns whether the save is needed plus the caller's value; the
    /// lock releases when the guard file drops.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the lock cannot be taken or the save
    /// fails.
    fn update<R>(&self, f: impl FnOnce(&mut State) -> (bool, R)) -> Result<R, Failure> {
        create_private_dirs(&self.root)?;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(false).mode(0o600);
        let lock = opts.open(self.state_lock_file())?;
        lock.lock()?;
        let mut state = self.load();
        let (dirty, out) = f(&mut state);
        if dirty {
            self.save(&state)?;
        }
        Ok(out)
    }

    /// Insert or replace the session for `device`.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn upsert(&self, device: &str, entry: &SessionEntry) -> Result<(), Failure> {
        self.update(|state| {
            state.devices.insert(device.to_owned(), entry.clone());
            (true, ())
        })
    }

    /// Drop the session for `device`; absent is not an error and skips the
    /// save entirely.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn remove(&self, device: &str) -> Result<(), Failure> {
        self.update(|state| (state.devices.remove(device).is_some(), ()))
    }

    /// Remember `device` as the default for future invocations (`--device`);
    /// an unchanged default skips the save.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn remember_device(&self, device: &str) -> Result<(), Failure> {
        self.update(|state| {
            if state.default_device.as_deref() == Some(device) {
                return (false, ());
            }
            state.default_device = Some(device.to_owned());
            (true, ())
        })
    }

    /// Deterministic token-file name for a device (see
    /// [`crate::secret::token_file_name`]).
    #[must_use]
    pub fn token_file_for(device: &str) -> String {
        token_file_name(device)
    }

    /// Write a session token through the secret helper at `0o600`.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the name is invalid or the write fails.
    pub fn write_token(&self, token_file: &str, token: &str) -> Result<(), Failure> {
        validate_token_file_name(token_file)?;
        write_secret(&self.token_path(token_file), token)
    }

    /// Read the token a session entry points at. The stored file name is
    /// validated first — a hand-edited or corrupt state file must not be
    /// able to steer this read outside `tokens/`.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the name is invalid or the file
    /// cannot be read.
    pub fn read_token(&self, entry: &SessionEntry) -> Result<String, Failure> {
        validate_token_file_name(&entry.token_file)?;
        let raw = fs::read_to_string(self.token_path(&entry.token_file))?;
        Ok(raw.trim_end().to_owned())
    }

    /// Remove a token file; a missing file is not an error. Required before
    /// every `write_token` for a returning device, since `write_secret`
    /// refuses to overwrite (KTD6).
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the name is invalid or removal fails.
    pub fn remove_token(&self, token_file: &str) -> Result<(), Failure> {
        validate_token_file_name(token_file)?;
        match fs::remove_file(self.token_path(token_file)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Failure::from(e)),
        }
    }

    /// Resolve the endpoint for one invocation, honoring the env overrides.
    /// `None` means no usable session — the caller may lazy-start a driver.
    /// A state entry whose recorded pid is dead resolves as absent (the file
    /// keeps the row so `serve` can still reap its orphaned runner).
    ///
    /// # Errors
    /// Returns [`Failure::Usage`] when exactly one override var is set and no
    /// state entry completes the pair.
    pub fn resolve(&self, device: Option<&str>) -> Result<Option<ResolvedEndpoint>, Failure> {
        self.resolve_with(device, env_val(URL_ENV), env_val(TOKEN_ENV))
    }

    /// `resolve` with the env values passed explicitly (the testable seam).
    ///
    /// # Errors
    /// Same contract as [`StateStore::resolve`].
    pub fn resolve_with(
        &self,
        device: Option<&str>,
        env_url: Option<String>,
        env_token: Option<String>,
    ) -> Result<Option<ResolvedEndpoint>, Failure> {
        let url_set = env_url.is_some();
        let token_set = env_token.is_some();
        let state = self.load();
        let name = device
            .map(String::from)
            .or_else(|| state.default_device.clone());
        let entry = name
            .as_ref()
            .and_then(|n| state.devices.get(n).cloned())
            .filter(|e| crate::process::pid_alive(e.pid));
        let url = env_url.or_else(|| entry.as_ref().map(|e| e.url.clone()));
        let token = env_token.or_else(|| entry.as_ref().and_then(|e| self.read_token(e).ok()));
        match (url, token) {
            (Some(url), Some(token)) => Ok(Some(ResolvedEndpoint {
                url,
                device: name,
                token,
            })),
            (None, _) if token_set => Err(Failure::usage(
                "AGENT_MOBILE_TOKEN has no driver URL to pair with; set AGENT_MOBILE_URL or run `agent-mobile serve`",
            )),
            (Some(_), None) if url_set => Err(Failure::usage(
                "AGENT_MOBILE_URL is set but no token is known; set AGENT_MOBILE_TOKEN or run `agent-mobile serve`",
            )),
            _ => Ok(None),
        }
    }
}

fn env_val(key: &str) -> Option<String> {
    std::env::var(key).ok()
}
