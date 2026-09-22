//! Error registry: the six wire codes, their HTTP statuses, the next-action
//! hint each prints, and the single exit-code mapping every verb inherits.

use crate::contract::ErrorBody;

/// Exit code for every failure that carries a wire code, plus the synthesized
/// transport and version-mismatch failures.
pub const EXIT_ERROR: i32 = 1;

/// Exit code for argument and usage failures caught before any round trip.
pub const EXIT_USAGE: i32 = 2;

/// Fixed stanza for a synthesized transport `DRIVER_ERROR` (KTD13): the wire
/// cannot distinguish refused, timed-out, untrusted, off-network, and
/// asleep-Mac failures, so one checklist names every developer check and
/// states that re-trust is human-only with no retry loop.
const TRANSPORT_ESCALATION: &str = concat!(
    "next: the driver is unreachable; work through this checklist:\n",
    "  - is `agent-mobile serve` running on the Mac? (any verb can lazy-start it)\n",
    "  - is the development certificate still trusted on the device?\n",
    "    re-trust lives in Settings > General > VPN & Device Management\n",
    "  - are the device and the Mac on the same Wi-Fi network?\n",
    "  - is the Mac awake? sleep drops the driver\n",
    "certificate re-trust is human-only; there is no retry loop"
);

/// The six codes the driver can return in `error.code` (PRD section 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The ref's snapshot id does not match, or no live element matches it.
    StaleRef,
    /// More than one live element matches the ref's identity evidence.
    AmbiguousTarget,
    /// A required body field is missing or a value is invalid.
    BadRequest,
    /// The path does not match a known verb.
    UnknownCommand,
    /// The bearer token is missing or wrong.
    Unauthorized,
    /// An error the driver did not anticipate.
    DriverError,
}

impl ErrorCode {
    /// Every code, in registry order.
    pub const ALL: [Self; 6] = [
        Self::StaleRef,
        Self::AmbiguousTarget,
        Self::BadRequest,
        Self::UnknownCommand,
        Self::Unauthorized,
        Self::DriverError,
    ];

    /// The verbatim wire code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StaleRef => "STALE_REF",
            Self::AmbiguousTarget => "AMBIGUOUS_TARGET",
            Self::BadRequest => "BAD_REQUEST",
            Self::UnknownCommand => "UNKNOWN_COMMAND",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::DriverError => "DRIVER_ERROR",
        }
    }

    /// The next-action hint the code prints in text mode.
    #[must_use]
    pub fn next_action(self) -> &'static str {
        match self {
            Self::StaleRef => "re-snapshot, then retry the action with a fresh ref",
            Self::AmbiguousTarget => {
                "pick a ref with a firmer native_id, walk --json bounds, or coordinate-tap from --json, then retry"
            }
            Self::BadRequest => "fix the request; do not retry unchanged",
            Self::UnknownCommand => "fix the client; do not retry",
            Self::Unauthorized => "fix AGENT_MOBILE_TOKEN; do not retry unchanged",
            Self::DriverError => "retry once; escalate if it recurs",
        }
    }

    /// Map a wire code string to the registry; unknown codes return `None`.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_str() == code)
    }
}

/// A renderable failure with a fixed exit code, so every verb maps to one of
/// these and the exit-code table lives in exactly one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The driver answered with an error envelope.
    Driver {
        /// Registry code the driver sent.
        code: ErrorCode,
        /// Driver-provided detail message.
        message: String,
    },
    /// No envelope at all: connection refused, timeout, or trust lapse.
    Transport {
        /// Transport-level detail, e.g. `connection refused`.
        message: String,
    },
    /// Usage failure caught before any round trip; exits 2.
    Usage {
        /// What was wrong with the invocation.
        message: String,
    },
    /// A local failure outside the wire contract, e.g. an unwritable state
    /// dir, a failed spawn, or a protocol version mismatch; exits 1.
    Local {
        /// What failed.
        message: String,
        /// What to do about it.
        next: String,
    },
}

impl Failure {
    /// Build a failure from a driver error body; unknown codes fall back to
    /// [`ErrorCode::DriverError`], the nearest fit.
    #[must_use]
    pub fn from_error_body(body: &ErrorBody) -> Self {
        let code = ErrorCode::from_code(&body.code).unwrap_or(ErrorCode::DriverError);
        Self::Driver {
            code,
            message: body.message.clone(),
        }
    }

    /// Build a failure for a known driver code with a message.
    #[must_use]
    pub fn driver(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Driver {
            code,
            message: message.into(),
        }
    }

    /// Synthesize the transport `DRIVER_ERROR`: no envelope exists, so the
    /// render carries the fixed escalation checklist instead of a hint.
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
        }
    }

    /// Build a usage failure; exits 2 before any round trip.
    #[must_use]
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
        }
    }

    /// Build a local (non-wire) failure with its own next action; exits 1.
    #[must_use]
    pub fn local(message: impl Into<String>, next: impl Into<String>) -> Self {
        Self::Local {
            message: message.into(),
            next: next.into(),
        }
    }

    /// Print the render to stderr and hand back the exit code — the pair
    /// every error exit performs. `eprintln!` panics on a closed stderr, so
    /// the write goes through a checked `writeln!` like stdout's `emit`.
    #[must_use]
    pub fn report(&self) -> i32 {
        use std::io::Write as _;
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "{}", self.render());
        let _ = err.flush();
        self.exit_code()
    }

    /// The failure message alone, without the rendered next-action stanza.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Driver { message, .. }
            | Self::Transport { message }
            | Self::Usage { message }
            | Self::Local { message, .. } => message,
        }
    }

    /// The `--json` failure path: stdout gets an envelope-shaped failure
    /// (`USAGE`/`LOCAL`/`DRIVER_ERROR` mark client-side failures that never
    /// reached the driver) and stderr keeps the human render.
    #[must_use]
    pub fn report_json(&self, command: &str) -> i32 {
        use std::io::Write as _;
        let code = match self {
            Self::Driver { code, .. } => code.as_str().to_owned(),
            Self::Transport { .. } => ErrorCode::DriverError.as_str().to_owned(),
            Self::Usage { .. } => "USAGE".to_owned(),
            Self::Local { .. } => "LOCAL".to_owned(),
        };
        let line = serde_json::json!({
            "version": crate::contract::PROTOCOL_VERSION,
            "ok": false,
            "command": command,
            "error": { "code": code, "message": self.message() },
        });
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{line}").is_err() || out.flush().is_err() {
            return 0;
        }
        drop(out);
        let _ = self.report();
        self.exit_code()
    }

    /// The single exit-code table: usage exits 2, everything else exits 1.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => EXIT_USAGE,
            Self::Driver { .. } | Self::Transport { .. } | Self::Local { .. } => EXIT_ERROR,
        }
    }

    /// Render the text-mode output: the verbatim code plus its next action.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Driver { code, message } => {
                let code_str = code.as_str();
                let hint = code.next_action();
                format!("{code_str}: {message}\nnext: {hint}")
            }
            Self::Transport { message } => {
                let code_str = ErrorCode::DriverError.as_str();
                format!("{code_str}: {message}\n{TRANSPORT_ESCALATION}")
            }
            Self::Usage { message } => {
                format!("usage: {message}\nnext: fix the command line and retry")
            }
            Self::Local { message, next } => {
                format!("error: {message}\nnext: {next}")
            }
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self::local(e.to_string(), "fix the local problem and retry")
    }
}
