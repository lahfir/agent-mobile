//! The verb registry (KTD1): every verb's name, help, arguments, global
//! options, hints, reply kind, and execution live once in [`VERBS`]; the
//! CLI ([`cli`]) and MCP ([`mcp`]) surfaces are projections of it.

pub mod cli;
pub mod kind;
pub mod mcp;
mod verbs;

use std::time::Duration;

use agent_mobile_core::error::Failure;
use agent_mobile_core::wire::LONG_TIMEOUT;
use serde_json::{Map, Value};

use crate::cmd::reply::Request;
use crate::cmd::{Call, Reply};

pub use verbs::VERBS;

/// Validated arguments keyed by registry name, shared by both surfaces.
pub type Args = Map<String, Value>;

/// One verb: everything both surfaces and dispatch need to know.
#[derive(Clone, Copy)]
pub struct Verb {
    /// Command and tool name.
    pub name: &'static str,
    /// One-line summary: clap `about`, MCP description head, guide line.
    pub summary: &'static str,
    /// Longer guidance appended to `--help` and the MCP description.
    pub guide: &'static str,
    /// Arguments in CLI positional order.
    pub args: &'static [Arg],
    /// Global options this verb honors.
    pub globals: &'static [Global],
    /// MCP behavior hints.
    pub hints: Hints,
    /// Reply shape, which selects the MCP output schema.
    pub reply: ReplyKind,
    /// How the verb runs.
    pub exec: Exec,
    /// Which surfaces expose the verb.
    pub surface: Surface,
}

/// One argument and its shared validation rule.
#[derive(Clone, Copy)]
pub struct Arg {
    /// Registry key; the MCP property name and the CLI flag stem.
    pub name: &'static str,
    /// Help text for both surfaces.
    pub help: &'static str,
    /// Type and bounds.
    pub kind: Kind,
    /// Whether the caller must supply it.
    pub required: bool,
    /// A `--flag` on the CLI instead of a positional.
    pub flag: bool,
}

/// Argument types; [`kind`] owns the one validator per kind (KTD2).
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Non-empty string up to this many characters.
    Text(usize),
    /// A `@<snapshot>:e<N>` ref.
    Ref,
    /// Finite number with optional bounds and a default.
    Number {
        /// Exclusive lower bound.
        gt: Option<f64>,
        /// Inclusive upper bound.
        max: Option<f64>,
        /// Value used when the caller omits it.
        default: Option<f64>,
    },
    /// Whole number that fits `u32`.
    Integer,
    /// One of a fixed set of strings.
    Enum(&'static [&'static str]),
    /// iOS bundle id or Android package.
    AppId,
    /// Local file path. CLI only: MCP never takes a path, so no tool call writes a file.
    Path,
    /// CLI `<REF> | <X> <Y>`; MCP `target` object `{ref}` or `{x, y}` (KTD3).
    Point,
    /// CLI `[REF] TEXT...`; MCP properties `ref` and `text` (KTD3).
    RefThenText,
}

/// Options shared across verbs; `--json` is CLI-only and applies to all.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Global {
    /// Device selector.
    Device,
    /// App selector.
    App,
    /// Snapshot depth limit.
    MaxDepth,
}

/// Every global, in the order both surfaces list them.
pub const GLOBALS: [Global; 3] = [Global::Device, Global::App, Global::MaxDepth];

impl Global {
    /// The global's argument definition.
    #[must_use]
    pub fn arg(self) -> Arg {
        let (name, help, kind) = match self {
            Self::Device => (
                "device",
                "Device to target: name, id, or platform:id key",
                Kind::Text(1024),
            ),
            Self::App => (
                "app",
                "Target app id (iOS bundle id / Android package)",
                Kind::AppId,
            ),
            Self::MaxDepth => (
                "max_depth",
                "Drop tree nodes deeper than N levels; marks the snapshot incomplete",
                Kind::Integer,
            ),
        };
        Arg {
            name,
            help,
            kind,
            required: false,
            flag: true,
        }
    }
}

/// The four MCP tool annotations.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the four MCP hints are independent protocol fields"
)]
#[derive(Clone, Copy)]
pub struct Hints {
    /// Reads only.
    pub read_only: bool,
    /// Can destroy state.
    pub destructive: bool,
    /// Repeating has no further effect.
    pub idempotent: bool,
    /// Touches state outside the server.
    pub open_world: bool,
}

/// Reads only, repeatable, local.
pub const READ: Hints = Hints {
    read_only: true,
    destructive: false,
    idempotent: true,
    open_world: false,
};
/// Acts on any target in the app, so destructive and not repeatable.
pub const ACTION: Hints = Hints {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: true,
};
/// Stops the app; a second stop changes nothing more.
pub const STOP: Hints = Hints {
    idempotent: true,
    ..ACTION
};

/// Reply shapes; each data shape has one MCP output schema (KTD6).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// Snapshot metadata plus the framed tree listing.
    Snapshot,
    /// Driver and device identity.
    Status,
    /// The terminated app.
    Terminate,
    /// Device rows.
    Devices,
    /// Plain text.
    Text,
    /// A PNG image.
    Image,
}

/// Which surfaces expose a verb.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// CLI and MCP.
    Both,
    /// CLI only: the verb blocks for a lifetime.
    Cli,
}

/// How a verb runs (KTD4).
#[derive(Clone, Copy)]
pub enum Exec {
    /// One driver round trip; the body is projected from the arguments.
    Wire {
        /// Driver route.
        route: &'static str,
        /// Wire budget rule.
        timeout: Timeout,
    },
    /// A host-side function.
    Local(fn(&Call) -> Result<Reply, Failure>),
}

/// Wire budget rules for actions that can outlast the default.
#[derive(Clone, Copy)]
pub enum Timeout {
    /// The standard budget.
    Default,
    /// Cold app starts and long text.
    Long,
    /// Press time plus settle and headroom, clamped so a huge value cannot wedge the client.
    Hold,
}

/// The verb named `name`.
#[must_use]
pub fn find(name: &str) -> Option<&'static Verb> {
    VERBS.iter().find(|verb| verb.name == name)
}

/// The driver request for a wire verb: every argument rides the body under
/// its registry name, a point target flattens to `ref` or `x`/`y`, and
/// `app_id` keeps its wire name `bundle_id` (KTD7).
#[must_use]
pub fn wire_request(verb: &Verb, args: &Args) -> Option<Request> {
    let Exec::Wire { route, timeout } = verb.exec else {
        return None;
    };
    let mut body = Map::new();
    for arg in verb.args {
        match (arg.kind, args.get(arg.name)) {
            (Kind::Point, Some(Value::Object(point))) => body.extend(point.clone()),
            (Kind::RefThenText, _) => {
                for key in ["ref", "text"] {
                    copy(args, &mut body, key, key);
                }
            }
            (_, Some(_)) => {
                let wire = if arg.name == "app_id" {
                    "bundle_id"
                } else {
                    arg.name
                };
                copy(args, &mut body, arg.name, wire);
            }
            (_, None) => {}
        }
    }
    if verb.globals.contains(&Global::App) {
        copy(args, &mut body, "app", "app");
    }
    let timeout = match timeout {
        Timeout::Default => None,
        Timeout::Long => Some(LONG_TIMEOUT),
        Timeout::Hold => {
            let press = args.get("duration").and_then(Value::as_f64).unwrap_or(1.0);
            Some(Duration::from_secs(30) + Duration::from_secs_f64(press.clamp(0.0, 60.0)))
        }
    };
    Some(Request {
        verb: route,
        body: Value::Object(body),
        timeout,
    })
}

fn copy(args: &Args, body: &mut Map<String, Value>, key: &str, wire: &str) {
    if let Some(value) = args.get(key) {
        body.insert(wire.to_owned(), value.clone());
    }
}

#[cfg(test)]
mod tests;
