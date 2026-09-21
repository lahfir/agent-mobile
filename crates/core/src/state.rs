//! Session state under `~/.agent-mobile` (KTD6, KTD7): which driver serves
//! which device, where each session's token file lives, and the env overrides
//! that win per invocation. A corrupt state file counts as stale, never fatal.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Failure;
use crate::secret::{create_private_dirs, write_secret};

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
/// it, when it started, and which token file holds the bearer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Base URL of the driver, e.g. `http://127.0.0.1:8770`.
    pub url: String,
    /// Pid of the process that owns the session.
    pub pid: u32,
    /// Unix seconds when the session was recorded.
    pub started_at: u64,
    /// Token file name inside `tokens/`; the token never appears here.
    pub token_file: String,
    /// Latest snapshot id the driver minted for this device, enabling local
    /// stale-ref rejection without a round trip.
    #[serde(default)]
    pub last_snapshot_id: Option<String>,
}

impl SessionEntry {
    /// Record a session that started now.
    #[must_use]
    pub fn new(url: String, pid: u32, token_file: String) -> Self {
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            url,
            pid,
            started_at,
            token_file,
            last_snapshot_id: None,
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

/// What an invocation resolved to: a live endpoint or nothing usable.
#[derive(Debug)]
pub enum ResolveOutcome {
    /// URL and token are both known.
    Ready(ResolvedEndpoint),
    /// No usable session; the caller may lazy-start or ask the user to serve.
    NoSession,
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
        let tmp = self.root.join(".state.json.tmp");
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

    /// Insert or replace the session for `device`.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn upsert(&self, device: &str, entry: &SessionEntry) -> Result<(), Failure> {
        let mut state = self.load();
        state.devices.insert(device.to_owned(), entry.clone());
        self.save(&state)
    }

    /// Drop the session for `device`; absent is not an error.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn remove(&self, device: &str) -> Result<(), Failure> {
        let mut state = self.load();
        state.devices.remove(device);
        self.save(&state)
    }

    /// Record the driver's newest snapshot id for `device`; absent entry is
    /// not an error.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn record_snapshot(&self, device: &str, snapshot_id: &str) -> Result<(), Failure> {
        let mut state = self.load();
        if let Some(entry) = state.devices.get_mut(device) {
            entry.last_snapshot_id = Some(snapshot_id.to_owned());
        }
        self.save(&state)
    }

    /// Remember `device` as the default for future invocations (`--device`).
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the state cannot be saved.
    pub fn remember_device(&self, device: &str) -> Result<(), Failure> {
        let mut state = self.load();
        state.default_device = Some(device.to_owned());
        self.save(&state)
    }

    /// Deterministic token-file name for a device: lowercased ASCII
    /// alphanumerics, everything else as `-`.
    #[must_use]
    pub fn token_file_for(device: &str) -> String {
        device
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect()
    }

    /// Write a session token through the secret helper at `0o600`.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the name is invalid or the write fails.
    pub fn write_token(&self, token_file: &str, token: &str) -> Result<(), Failure> {
        validate_token_file(token_file)?;
        write_secret(&self.token_path(token_file), token)
    }

    /// Read the token a session entry points at.
    ///
    /// # Errors
    /// Returns [`Failure::Local`] when the file cannot be read.
    pub fn read_token(&self, entry: &SessionEntry) -> Result<String, Failure> {
        let raw = fs::read_to_string(self.token_path(&entry.token_file))?;
        Ok(raw.trim_end().to_owned())
    }

    /// Resolve the endpoint for one invocation, honoring the env overrides.
    ///
    /// # Errors
    /// Returns [`Failure::Usage`] when exactly one override var is set and no
    /// state entry completes the pair.
    pub fn resolve(&self, device: Option<&str>) -> Result<ResolveOutcome, Failure> {
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
    ) -> Result<ResolveOutcome, Failure> {
        let url_set = env_url.is_some();
        let token_set = env_token.is_some();
        let state = self.load();
        let name = device
            .map(String::from)
            .or_else(|| state.default_device.clone());
        let entry = name.as_ref().and_then(|n| state.devices.get(n).cloned());
        let url = env_url.or_else(|| entry.as_ref().map(|e| e.url.clone()));
        let token = env_token.or_else(|| entry.and_then(|e| self.read_token(&e).ok()));
        match (url, token) {
            (Some(url), Some(token)) => Ok(ResolveOutcome::Ready(ResolvedEndpoint {
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
            _ => Ok(ResolveOutcome::NoSession),
        }
    }
}

fn env_val(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn validate_token_file(name: &str) -> Result<(), Failure> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Failure::local(
            format!("invalid token file name {name:?}"),
            "use the session store API to mint token file names",
        ))
    }
}
