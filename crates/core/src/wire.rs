//! The wire client (KTD2, KTD3): one configured ureq agent speaks the driver
//! protocol — bearer, close, and version headers on every call, a 30 s global
//! timeout since the driver has none, and status-as-error off so error
//! envelopes survive to the parser.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::contract::{Envelope, PROTOCOL_VERSION};
use crate::error::{ErrorCode, Failure};

/// Default ceiling for one call; the driver has no timeout of its own.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A well-formed driver reply: the typed envelope plus the raw body so
/// `--json` passes bytes through untouched, including error replies.
pub struct Reply {
    /// Parsed envelope.
    pub envelope: Envelope,
    /// Raw reply body as sent by the driver.
    pub raw: String,
}

/// Configured client bound to one driver base URL and bearer token.
pub struct Wire {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl Wire {
    /// Client with the standard 30 s global timeout.
    #[must_use]
    pub fn new(base: &str, token: &str) -> Self {
        Self::with_timeout(base, token, DEFAULT_TIMEOUT)
    }

    /// Client with an explicit global timeout; tests use short ones.
    #[must_use]
    pub fn with_timeout(base: &str, token: &str, timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build();
        let agent = ureq::Agent::new_with_config(config);
        Self {
            agent,
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        }
    }

    /// `POST /<verb>` with a JSON body. Any well-formed envelope — success or
    /// error — comes back `Ok`; only transport failures, unparseable bodies,
    /// and version mismatches are `Err`.
    ///
    /// # Errors
    /// [`Failure::Transport`] on refused, timed-out, or other transport
    /// failures; [`Failure::VersionMismatch`] on a non-`"1"` envelope version;
    /// [`ErrorCode::DriverError`] on an unparseable reply body.
    pub fn call(&self, verb: &str, body: &Value) -> Result<Reply, Failure> {
        let url = format!("{}/{verb}", self.base);
        let mut resp = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Connection", "close")
            .header("X-Agent-Mobile-Version", PROTOCOL_VERSION)
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| Failure::transport(e.to_string()))?;
        let mut buf = Vec::new();
        resp.body_mut()
            .as_reader()
            .read_to_end(&mut buf)
            .map_err(|e| Failure::transport(e.to_string()))?;
        let raw = String::from_utf8_lossy(&buf).into_owned();
        let envelope = Envelope::from_json(&raw).map_err(|e| {
            Failure::driver(
                ErrorCode::DriverError,
                format!("driver sent an unparseable reply: {e}"),
            )
        })?;
        envelope.check_version()?;
        Ok(Reply { envelope, raw })
    }
}
