//! JSON-RPC envelope validation and flushed newline framing.

use std::io::Write;

use serde_json::{Value, json};

/// A checked request or notification; a missing id marks a notification.
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// Parse one line, retaining a valid id for invalid-request errors.
pub fn parse(line: &[u8]) -> Result<Request, Value> {
    let value: Value =
        serde_json::from_slice(line).map_err(|_| error(&Value::Null, -32700, "Parse error"))?;
    let id = value.get("id").cloned();
    let valid_id = id
        .as_ref()
        .is_none_or(|id| id.is_string() || id.as_i64().is_some() || id.as_u64().is_some());
    if value["jsonrpc"] != "2.0" || !value["method"].is_string() || !valid_id {
        return Err(error(
            &if valid_id {
                id.unwrap_or(Value::Null)
            } else {
                Value::Null
            },
            -32600,
            "Invalid Request",
        ));
    }
    Ok(Request {
        id,
        method: value["method"].as_str().unwrap_or_default().to_owned(),
        params: value.get("params").cloned().unwrap_or_else(|| json!({})),
    })
}

/// A protocol fault, distinct from a successfully delivered tool failure.
pub fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}})
}

/// A successful RPC envelope, including tool results with `isError:true`.
pub fn success(id: &Value, result: &Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "result":result})
}

/// A closed stdout is a normal client disconnect.
pub fn write(out: &mut impl Write, value: &Value) -> std::io::Result<()> {
    writeln!(out, "{value}")?;
    out.flush()
}
