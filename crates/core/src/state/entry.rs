//! Session-entry and resolved-endpoint types: the on-disk row one live
//! driver writes, plus the URL+token pair a verb resolves (KTD6, KTD11).

use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_pid: Option<u32>,
    /// `ios` or `android`; absent on P1 rows means iOS (KTD11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Stable device id (UDID, serial, or `avd:<name>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// `adb` serial an Android session is bound to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Owned `adb forward` host port (Android only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_port: Option<u16>,
    /// Local bridge listen port (Android only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_port: Option<u16>,
    /// Device-side loopback port the forward targets (Android only); old
    /// rows without it are the legacy fixed port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_port: Option<u16>,
    /// APK installed for the session (Android only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_source: Option<String>,
    /// Emulator pid spawned by this serve, when it booted one (0 = reused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator_pid: Option<u32>,
    /// Driver/emulator log path for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    /// [`crate::process::process_identity`] marker for `pid`, pinned at
    /// record time so a recycled pid can never impersonate this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_started_at: Option<String>,
    /// Display name of the device this session serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
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
            platform: None,
            device_id: None,
            serial: None,
            forward_port: None,
            bridge_port: None,
            device_port: None,
            apk_source: None,
            emulator_pid: None,
            log_file: None,
            process_started_at: crate::process::process_identity(pid),
            device_name: None,
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
    /// Construct one endpoint; `state` is the only caller.
    pub(crate) fn new(url: String, device: Option<String>, token: String) -> Self {
        Self { url, device, token }
    }

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
