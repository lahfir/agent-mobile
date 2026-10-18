//! The MCP projection: `tools/list` and `tools/call` argument handling,
//! built from [`VERBS`]. Argument faults are tool errors; only an unknown
//! tool or a malformed call is a protocol error (R10).

use serde_json::{Map, Value, json};

use super::kind::{self, Fault};
use super::{Arg, Kind, ReplyKind, Surface, VERBS, Verb, find};
use crate::cmd::Call;

const REFS: &str = "Refs come from the most recent snapshot or action reply and die after every action; on STALE_REF, take a new snapshot.";

/// Why a `tools/call` request was refused before execution.
pub enum Rejected {
    /// A JSON-RPC `-32602` with this message.
    Protocol(&'static str),
    /// An `isError` tool result.
    Tool(Fault),
}

/// The `tools/list` result.
#[must_use]
pub fn tools() -> Value {
    let tools: Vec<Value> = VERBS
        .iter()
        .filter(|v| v.surface == Surface::Both)
        .map(tool)
        .collect();
    json!({"tools": tools, "ttlMs": 86_400_000, "cacheScope": "public"})
}

fn tool(verb: &Verb) -> Value {
    let h = verb.hints;
    let mut tool = json!({
        "name": verb.name,
        "description": description(verb),
        "inputSchema": input_schema(verb),
        "annotations": {
            "readOnlyHint": h.read_only,
            "destructiveHint": h.destructive,
            "idempotentHint": h.idempotent,
            "openWorldHint": h.open_world,
        },
    });
    if let Some(schema) = output_schema(verb.reply) {
        tool["outputSchema"] = schema;
    }
    tool
}

fn description(verb: &Verb) -> String {
    let mut text = verb.summary.to_owned();
    if !verb.guide.is_empty() {
        text = format!("{text}. {}", verb.guide);
    }
    if verb
        .args
        .iter()
        .any(|a| matches!(a.kind, Kind::Ref | Kind::Point | Kind::RefThenText))
    {
        text = format!("{text} {REFS}");
    }
    text
}

fn input_schema(verb: &Verb) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let args = verb.args.iter().copied().filter(|a| a.kind != Kind::Path);
    for arg in args.chain(verb.globals.iter().map(|g| g.arg())) {
        if arg.kind == Kind::RefThenText {
            properties.insert(
                "ref".to_owned(),
                string(
                    1024,
                    "Ref of the field to write; omitted types into the focused field",
                ),
            );
            properties.insert(
                "text".to_owned(),
                json!({"type": "string", "maxLength": 65_536, "description": arg.help}),
            );
            required.push("text");
            continue;
        }
        properties.insert(arg.name.to_owned(), schema(&arg));
        if arg.required {
            required.push(arg.name);
        }
    }
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn string(max: usize, help: &str) -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": max, "description": help})
}

fn schema(arg: &Arg) -> Value {
    let mut out = match arg.kind {
        Kind::Text(max) => string(max, arg.help),
        Kind::Ref | Kind::Path | Kind::RefThenText => string(1024, arg.help),
        Kind::AppId => {
            json!({"type": "string", "minLength": 1, "maxLength": 1024, "pattern": "^[A-Za-z0-9._-]+$"})
        }
        Kind::Number { gt, max, default } => {
            let mut n = json!({"type": "number"});
            if let Some(v) = gt {
                n["exclusiveMinimum"] = json!(v);
            }
            if let Some(v) = max {
                n["maximum"] = json!(v);
            }
            if let Some(v) = default {
                n["default"] = json!(v);
            }
            n
        }
        Kind::Integer => json!({"type": "integer", "minimum": 0, "maximum": u32::MAX}),
        Kind::Enum(values) => json!({"type": "string", "enum": values}),
        Kind::Point => json!({"oneOf": [
            {"type": "object", "properties": {"ref": string(1024, "Ref from the most recent snapshot or action reply")},
             "required": ["ref"], "additionalProperties": false},
            {"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}},
             "required": ["x", "y"], "additionalProperties": false},
        ]}),
    };
    out["description"] = json!(arg.help);
    out
}

/// The output schema for a reply kind; text and image replies have none.
#[must_use]
pub fn output_schema(reply: ReplyKind) -> Option<Value> {
    let object = |fields: &[(&str, &str)], required: &[&str]| {
        let properties: Map<String, Value> = fields
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), json!({"type": ty})))
            .collect();
        json!({"type": "object", "properties": properties, "required": required})
    };
    match reply {
        ReplyKind::Snapshot => Some(object(
            &[
                ("app", "string"),
                ("snapshot_id", "string"),
                ("ref_count", "integer"),
                ("complete", "boolean"),
                ("settled", "boolean"),
                ("reads", "integer"),
                ("settle_ms", "integer"),
            ],
            &[
                "app",
                "snapshot_id",
                "ref_count",
                "complete",
                "settled",
                "reads",
            ],
        )),
        ReplyKind::Status => Some(object(
            &[
                ("app", "string"),
                ("snapshot_id", "string"),
                ("device", "string"),
                ("os", "string"),
            ],
            &["app", "snapshot_id", "device", "os"],
        )),
        ReplyKind::Terminate => Some(object(&[("terminated", "string")], &["terminated"])),
        ReplyKind::Devices => {
            let mut row = object(
                &[
                    ("key", "string"),
                    ("name", "string"),
                    ("platform", "string"),
                    ("kind", "string"),
                    ("os", "string"),
                    ("state", "string"),
                    ("serving", "string"),
                ],
                &["key", "name", "platform", "kind", "os", "state", "serving"],
            );
            for field in ["os", "state", "serving"] {
                row["properties"][field]["type"] = json!(["string", "null"]);
            }
            Some(
                json!({"type": "object", "properties": {"devices": {"type": "array", "items": row}}, "required": ["devices"]}),
            )
        }
        ReplyKind::Text | ReplyKind::Image => None,
    }
}

/// Turn `tools/call` params into a validated call.
///
/// # Errors
/// [`Rejected::Protocol`] for an unknown tool or malformed params,
/// [`Rejected::Tool`] for any argument fault.
pub fn prepare(params: &Value) -> Result<Call, Rejected> {
    let verb = params["name"]
        .as_str()
        .and_then(find)
        .filter(|v| v.surface == Surface::Both)
        .ok_or(Rejected::Protocol("Unknown tool"))?;
    if params.as_object().is_none_or(|p| {
        p.keys()
            .any(|k| !matches!(k.as_str(), "name" | "arguments" | "_meta"))
    }) {
        return Err(Rejected::Protocol("Invalid tools/call parameters"));
    }
    let empty = Map::new();
    let input = match params.get("arguments") {
        None => &empty,
        Some(Value::Object(map)) => map,
        Some(_) => {
            return Err(Rejected::Tool(Fault {
                field: "arguments".to_owned(),
                problem: "must be an object".to_owned(),
                fix: "pass the tool's arguments as a JSON object".to_owned(),
            }));
        }
    };
    let args = kind::check_args(verb, input, false).map_err(Rejected::Tool)?;
    Ok(Call {
        verb,
        args,
        json: false,
        remember: false,
    })
}
