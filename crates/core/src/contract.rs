//! Wire contract shared with the driver: the reply envelope, the per-verb
//! `data` shapes, and the accessibility `Node` tree.

use serde::{Deserialize, Serialize};

use crate::error::Failure;

/// Protocol version both sides must speak; fixed at `"1"` for P1 and bumped
/// only on a breaking change.
pub const PROTOCOL_VERSION: &str = "1";

/// Reply envelope for every verb. `ok` discriminates: `true` carries `data`,
/// `false` carries `error`. A 401 arrives before dispatch and omits `command`
/// and `elapsed_ms`, so both stay optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol version the driver speaks; must equal [`PROTOCOL_VERSION`].
    pub version: String,
    /// `true` on success, `false` on failure.
    pub ok: bool,
    /// Echoed verb; absent on a 401 reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Server-side milliseconds the call took; absent on a 401 reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Per-verb payload; present when `ok` is `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Data>,
    /// `{code, message}` pair; present when `ok` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Envelope {
    /// Parse one raw reply body into the typed envelope.
    ///
    /// # Errors
    /// Returns the `serde_json` error when the body is not a valid envelope.
    pub fn from_json(body: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(body)
    }

    /// Fatal upgrade check: the driver's `version` must equal
    /// [`PROTOCOL_VERSION`].
    ///
    /// # Errors
    /// Returns [`Failure::Local`] on any other version.
    pub fn check_version(&self) -> Result<(), Failure> {
        if self.version == PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(Failure::local(
                format!(
                    "version mismatch: driver speaks protocol \"{}\"; \
                     this CLI requires \"{PROTOCOL_VERSION}\"",
                    self.version
                ),
                format!(
                    "upgrade agent-mobile and the driver together; \
                     both must speak protocol version \"{PROTOCOL_VERSION}\""
                ),
            ))
        }
    }
}

/// The `{code, message}` pair inside a failed envelope. `code` stays a raw
/// string so an unknown code still parses; the registry maps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Verbatim wire code, e.g. `STALE_REF`.
    pub code: String,
    /// Driver-provided detail message.
    pub message: String,
}

/// Per-verb `data` payload. Untagged and ordered most-distinctive-first so
/// serde tries the richest shape before the sparse ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Data {
    /// Settled snapshot returned by `snapshot`, `launch`, `tap`,
    /// `type`, `swipe`, and `home`.
    Snapshot(Box<Snapshot>),
    /// `status` reply: identity and device info, no tree, no settle.
    Status(Status),
    /// `terminate` reply: the bundle that was terminated.
    Terminate(Terminate),
    /// `screenshot` reply: base64 PNG bytes.
    Screenshot(Screenshot),
}

/// Settled snapshot payload: the active app, the snapshot identity, settle
/// bookkeeping, the pre-rendered text listing, and the node tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Active bundle id the snapshot was taken against.
    pub app: String,
    /// Snapshot id that qualifies refs as `@<snapshot_id>:e<N>`.
    #[allow(
        clippy::struct_field_names,
        reason = "the field mirrors the driver's fixed wire key `snapshot_id`"
    )]
    pub snapshot_id: String,
    /// Number of refs minted for this snapshot.
    pub ref_count: u64,
    /// `false` when the core pruned nodes below `--max-depth`.
    pub complete: bool,
    /// `true` when the settle loop observed two identical reads.
    pub settled: bool,
    /// How many tree reads the settle loop performed.
    pub reads: u64,
    /// Milliseconds the settle loop took; absent from older drivers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_ms: Option<u64>,
    /// Driver-rendered text listing of named or interactive nodes.
    pub text: String,
    /// Root accessibility node.
    pub tree: Node,
}

/// `status` payload: no tree and no settle step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Active bundle id.
    pub app: String,
    /// Last minted snapshot id, or empty before the first snapshot.
    pub snapshot_id: String,
    /// Device name.
    pub device: String,
    /// OS version string.
    pub os: String,
}

/// `terminate` payload: the only reply that is not a settled snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terminate {
    /// Bundle id that was terminated.
    pub terminated: String,
}

/// `screenshot` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    /// PNG bytes encoded as base64.
    pub png_base64: String,
}

/// One accessibility node in the snapshot tree. Every node carries a `ref_id`
/// because mobile trees have tappable unnamed containers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Element role, e.g. `button` or `textfield`.
    pub role: String,
    /// Accessibility label or title; may be empty.
    pub name: String,
    /// Element value; may be empty.
    pub value: String,
    /// Per-snapshot ref in the form `@<snapshot_id>:e<N>`.
    pub ref_id: String,
    /// State tags like `disabled`, `selected`, `focused`.
    pub states: Vec<String>,
    /// Actions the element supports, e.g. `Tap`, `Type`, `Swipe`.
    pub available_actions: Vec<String>,
    /// Frame in points inside the app window.
    pub bounds: Bounds,
    /// Stable identifier when the element exposes one (`ax_identifier` in P1).
    pub native_id: Option<NativeId>,
    /// Child nodes.
    pub children: Vec<Node>,
}

/// Element frame `{x, y, width, height}` in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// Left edge offset in points.
    pub x: f64,
    /// Top edge offset in points.
    pub y: f64,
    /// Width in points.
    pub width: f64,
    /// Height in points.
    pub height: f64,
}

/// Stable native identifier for an element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeId {
    /// Identifier kind; `ax_identifier` in P1.
    pub kind: String,
    /// Identifier value.
    pub value: String,
}

/// A parsed `@<snapshot_id>:e<N>` element ref (R3). Refs are minted per
/// snapshot and die with it; the driver rejects a superseded ref with
/// `STALE_REF` — freshness is checked against the live tree, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// Snapshot id the ref was minted under.
    pub snapshot_id: String,
    /// Sequence number inside that snapshot.
    pub index: u64,
}

impl Ref {
    /// Parse `@<id>:e<N>`; anything else is a usage error caught before any
    /// round trip.
    ///
    /// # Errors
    /// Returns [`Failure::Usage`] when `s` is not a well-formed ref.
    pub fn parse(s: &str) -> Result<Self, Failure> {
        let rest = s.strip_prefix('@').ok_or_else(|| bad_ref(s))?;
        let (id, seq) = rest.split_once(':').ok_or_else(|| bad_ref(s))?;
        let index = seq
            .strip_prefix('e')
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|_| !id.is_empty())
            .ok_or_else(|| bad_ref(s))?;
        Ok(Self {
            snapshot_id: id.to_owned(),
            index,
        })
    }
}

/// The malformed-ref usage error; one message, three rejection sites.
fn bad_ref(s: &str) -> Failure {
    Failure::usage(format!("ref must look like @<snapshot>:e<N>; got {s:?}"))
}

impl std::fmt::Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@{}:e{}", self.snapshot_id, self.index)
    }
}

/// Drop nodes deeper than `max_depth` (root is depth 0) and mark the
/// snapshot incomplete; the driver always sends the full tree, so trimming
/// is client-side (R9). Returns `true` when nodes were actually pruned —
/// the caller regenerates `snap.text` so it stays consistent with `tree`.
pub fn trim_snapshot(snap: &mut Snapshot, max_depth: u32) -> bool {
    let pruned = trim_node(&mut snap.tree, max_depth, 0);
    if pruned {
        snap.complete = false;
    }
    pruned
}

fn trim_node(node: &mut Node, max_depth: u32, depth: u32) -> bool {
    if depth >= max_depth {
        let had = !node.children.is_empty();
        node.children.clear();
        return had;
    }
    node.children.iter_mut().fold(false, |pruned, c| {
        trim_node(c, max_depth, depth + 1) | pruned
    })
}
