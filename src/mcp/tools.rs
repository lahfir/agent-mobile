//! Fixed tool catalog and argument validation before any session lookup.

use agent_mobile_core::error::Failure;
use serde_json::{Value, json};

use crate::cmd::{self, reply::Request};

pub const NAMES: [&str; 18] = [
    "devices",
    "status",
    "snapshot",
    "tap",
    "type",
    "swipe",
    "scroll_until_visible",
    "home",
    "doubletap",
    "pinch",
    "hold",
    "back",
    "twofinger",
    "center",
    "launch",
    "screenshot",
    "stop",
    "skills",
];

/// Validated arguments and shared CLI request, ready for the serial worker.
pub struct Call {
    pub name: String,
    pub request: Option<Request>,
    pub app: Option<String>,
    pub max_depth: Option<u32>,
}

pub fn read_only(name: &str) -> bool {
    matches!(
        name,
        "devices" | "status" | "snapshot" | "screenshot" | "skills"
    )
}

fn schema(name: &str) -> Value {
    let string = json!({"type":"string", "minLength":1, "maxLength":1024});
    let reference = json!({"type":"string", "minLength":1, "maxLength":1024,
        "description":"Ref from the most recent snapshot or action reply"});
    let number = json!({"type":"number"});
    let mut properties = json!({});
    let mut required: Vec<&str> = Vec::new();
    match name {
        "snapshot" => {
            properties["app"] = string.clone();
            properties["max_depth"] = json!({"type":"integer", "minimum":0, "maximum":u32::MAX});
        }
        "tap" | "doubletap" | "hold" => {
            properties = json!({"ref":reference, "x":number, "y":number});
            if name == "hold" {
                properties["duration"] =
                    json!({"type":"number", "exclusiveMinimum":0, "maximum":10, "default":1.0});
            }
        }
        "type" => {
            properties = json!({"ref":reference, "text":{"type":"string", "maxLength":65536}});
            required.push("text");
        }
        "swipe" => {
            properties = json!({"ref":reference, "direction":{"type":"string", "enum":["up","down","left","right"]}});
            required.push("direction");
        }
        "scroll_until_visible" => {
            properties["text"] = json!({"type":"string", "minLength":1, "maxLength":1024});
            properties["direction"] =
                json!({"type":"string", "enum":["up","down","left","right"], "default":"up"});
            required.push("text");
        }
        "pinch" => {
            properties = json!({"ref":reference, "scale":{"type":"number", "exclusiveMinimum":0}, "velocity":number});
            required.extend(["ref", "scale"]);
        }
        "twofinger" => {
            properties["ref"] = reference;
            required.push("ref");
        }
        "center" => {
            properties["which"] = json!({"type":"string", "enum":["notification"]});
            required.push("which");
        }
        "launch" => {
            properties["bundle_id"] = string;
            required.push("bundle_id");
        }
        _ => {}
    }
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}

fn description(name: &str) -> String {
    let text = match name {
        "devices" => "List reachable devices; does not start a driver.",
        "status" => "Show the active app, device and snapshot identity.",
        "snapshot" => {
            "Capture the accessibility tree and mint fresh refs; app selects the target, max_depth trims deep nodes."
        }
        "tap" => "Tap a ref or x,y point in app-frame coordinates. Supply ref OR both x and y.",
        "type" => "Append text to a ref or the focused field; returns the next snapshot.",
        "swipe" => "Swipe in a direction on a ref or the whole app; returns the next snapshot.",
        "scroll_until_visible" => {
            "Swipe until an exact node name appears; direction defaults to up."
        }
        "home" => "Go to the launcher/home and return its snapshot.",
        "doubletap" => "Tap twice on a ref or x,y app-frame point. Supply ref OR both x and y.",
        "pinch" => {
            "Zoom on a ref: scale above 1 zooms out, below 1 zooms in; near-1 scales fail. Returns the next snapshot."
        }
        "hold" => {
            "Press a ref or x,y app-frame point for 0 < duration <= 10 seconds (default 1). Supply ref OR both x and y."
        }
        "back" => "Navigate back and return the next snapshot.",
        "twofinger" => "Two-finger tap on a ref; returns the next snapshot.",
        "center" => {
            "Open notifications in the system UI session; earlier refs die. Snapshot to return to your app; on Android, back out first."
        }
        "launch" => {
            "Cold-start or restart bundle_id, retaining app data but losing transient state; returns the next snapshot."
        }
        "screenshot" => "Capture a PNG image; takes no arguments and writes no file.",
        "stop" => "Terminate the active app; driver stays up. Snapshot to verify.",
        _ => "Read the agent guide.",
    };
    if matches!(
        name,
        "tap" | "doubletap" | "hold" | "pinch" | "type" | "swipe" | "twofinger"
    ) {
        format!(
            "{text} Ref must come from the most recent snapshot or action reply; returns the next snapshot."
        )
    } else {
        text.to_owned()
    }
}

/// Every schema is a closed plain object; ordering is a public discovery contract.
pub fn list() -> Value {
    let tools: Vec<Value> = NAMES
        .iter()
        .map(|name| {
            let read = read_only(name);
            json!({"name":name, "description":description(name), "inputSchema":schema(name),
            "annotations":{"readOnlyHint":read, "destructiveHint":!read,
                "idempotentHint":read || *name == "stop", "openWorldHint":!read}})
        })
        .collect();
    json!({"tools":tools, "ttlMs":86_400_000, "cacheScope":"public"})
}

fn invalid() -> Failure {
    Failure::usage("invalid tool arguments; follow the input schema")
}

fn validate(name: &str, args: &Value) -> Result<(), Failure> {
    let object = args.as_object().ok_or_else(invalid)?;
    let schema = schema(name);
    let properties = schema["properties"].as_object().ok_or_else(invalid)?;
    for (key, value) in object {
        let rule = properties.get(key).ok_or_else(invalid)?;
        let valid = match rule["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|s| {
                let len = s.chars().count() as u64;
                len >= rule["minLength"].as_u64().unwrap_or(0)
                    && len <= rule["maxLength"].as_u64().unwrap_or(u64::MAX)
            }),
            Some("number") => value.as_f64().is_some_and(|n| {
                n.is_finite()
                    && rule["exclusiveMinimum"].as_f64().is_none_or(|v| n > v)
                    && rule["maximum"].as_f64().is_none_or(|v| n <= v)
            }),
            Some("integer") => value.as_u64().is_some_and(|n| u32::try_from(n).is_ok()),
            _ => false,
        };
        if !valid
            || rule["enum"]
                .as_array()
                .is_some_and(|allowed| !allowed.contains(value))
        {
            return Err(invalid());
        }
    }
    if schema["required"].as_array().is_some_and(|keys| {
        keys.iter()
            .any(|key| !object.contains_key(key.as_str().unwrap_or_default()))
    }) {
        return Err(invalid());
    }
    Ok(())
}

fn identifier(value: &str) -> Result<(), Failure> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(invalid());
    }
    Ok(())
}

fn point(args: &Value) -> Result<Vec<String>, Failure> {
    match (args["ref"].as_str(), args.get("x"), args.get("y")) {
        (Some(reference), None, None) => Ok(vec![reference.to_owned()]),
        (None, Some(x), Some(y)) => Ok(vec![x.to_string(), y.to_string()]),
        _ => Err(invalid()),
    }
}

/// Validate all tool parameters, then call the same request builders as the CLI.
pub fn prepare(params: &Value) -> Result<Call, Failure> {
    let name = params["name"]
        .as_str()
        .filter(|name| NAMES.contains(name))
        .ok_or_else(invalid)?;
    if params.as_object().is_none_or(|p| {
        p.keys()
            .any(|k| !matches!(k.as_str(), "name" | "arguments" | "_meta"))
    }) {
        return Err(invalid());
    }
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);
    validate(name, args)?;
    let reference = args["ref"].as_str();
    let request = match name {
        "devices" | "skills" => None,
        "status" => Some(cmd::status::request()),
        "snapshot" => {
            if let Some(app) = args["app"].as_str() {
                identifier(app)?;
            }
            Some(cmd::snapshot::request(args["app"].as_str()))
        }
        "tap" => Some(cmd::tap::request(&point(args)?)?),
        "doubletap" => Some(cmd::doubletap::request(&point(args)?)?),
        "hold" => Some(cmd::hold::request(
            &point(args)?,
            args["duration"].as_f64().unwrap_or(1.0),
        )?),
        "type" => {
            let text = args["text"].as_str().ok_or_else(invalid)?;
            let mut request =
                cmd::r#type::request(&[reference.unwrap_or_default().to_owned(), text.to_owned()])?;
            request.body["text"] = json!(text);
            Some(request)
        }
        "swipe" => Some(cmd::swipe::request(
            args["direction"].as_str().ok_or_else(invalid)?,
            reference,
        )?),
        "scroll_until_visible" => {
            let direction = args["direction"].as_str().unwrap_or("up");
            let mut request = cmd::scroll::request(direction)?;
            request.body["text"] = args["text"].clone();
            Some(request)
        }
        "pinch" => {
            let scale = args["scale"].as_f64().ok_or_else(invalid)?;
            if (scale - 1.0).abs() < 0.01 {
                return Err(invalid());
            }
            Some(cmd::pinch::request(
                reference.ok_or_else(invalid)?,
                scale,
                args["velocity"].as_f64(),
            )?)
        }
        "twofinger" => Some(cmd::twofinger::request(reference.ok_or_else(invalid)?)?),
        "home" => Some(cmd::home::request()),
        "back" => Some(cmd::back::request()),
        "center" => Some(cmd::center::request(
            args["which"].as_str().ok_or_else(invalid)?,
        )),
        "launch" => {
            let id = args["bundle_id"].as_str().ok_or_else(invalid)?;
            identifier(id)?;
            Some(cmd::launch::request(id))
        }
        "screenshot" => Some(cmd::screenshot::request()),
        "stop" => Some(cmd::stop::request()),
        _ => return Err(invalid()),
    };
    Ok(Call {
        name: name.to_owned(),
        request,
        app: args["app"].as_str().map(str::to_owned),
        max_depth: args["max_depth"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_arguments_preserve_cli_payloads_and_literal_ref_shaped_text() -> Result<(), Failure> {
        let cases = [
            (
                "tap",
                json!({"x":12.5,"y":9}),
                "tap",
                json!({"x":12.5,"y":9.0}),
            ),
            (
                "type",
                json!({"text":"@snap1:e2"}),
                "type",
                json!({"text":"@snap1:e2"}),
            ),
            (
                "type",
                json!({"ref":"@snap1:e2","text":"-a\nb"}),
                "type",
                json!({"ref":"@snap1:e2","text":"-a\nb"}),
            ),
            (
                "hold",
                json!({"ref":"@snap1:e2"}),
                "hold",
                json!({"ref":"@snap1:e2","duration":1.0}),
            ),
            ("stop", json!({}), "terminate", json!({})),
        ];
        for (name, arguments, verb, expected) in cases {
            let call = prepare(&json!({"name":name,"arguments":arguments}))?;
            let request = call.request.ok_or_else(invalid)?;
            assert_eq!(request.verb, verb);
            assert_eq!(request.body, expected);
        }
        Ok(())
    }
}
