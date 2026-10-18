//! Registry projections: wire bodies, the clap tree, and the MCP catalog.

use std::time::Duration;

use serde_json::{Value, json};

use super::mcp::{Rejected, prepare, tools};
use super::{Surface, VERBS, find, wire_request};
use crate::cmd::reply::Request;

fn mcp(name: &str, arguments: &Value) -> Option<Request> {
    let call = prepare(&json!({"name": name, "arguments": arguments})).ok()?;
    wire_request(call.verb, &call.args)
}

fn listed(name: &str) -> Option<Value> {
    tools()["tools"]
        .as_array()
        .and_then(|t| t.iter().find(|t| t["name"] == name).cloned())
}

#[test]
fn generated_clap_command_is_valid() {
    let command = super::cli::command();
    command.clone().debug_assert();
    assert_eq!(command.get_subcommands().count(), VERBS.len());
}

#[test]
fn named_arguments_preserve_wire_payloads_and_literal_ref_shaped_text() {
    let cases = [
        (
            "tap",
            json!({"target": {"x": 12.5, "y": 9}}),
            "tap",
            json!({"x": 12.5, "y": 9.0}),
        ),
        (
            "type",
            json!({"text": "@snap1:e2"}),
            "type",
            json!({"text": "@snap1:e2"}),
        ),
        (
            "type",
            json!({"ref": "@snap1:e2", "text": "-a\nb"}),
            "type",
            json!({"ref": "@snap1:e2", "text": "-a\nb"}),
        ),
        (
            "hold",
            json!({"target": {"ref": "@snap1:e2"}}),
            "hold",
            json!({"ref": "@snap1:e2", "duration": 1.0}),
        ),
        ("stop", json!({}), "terminate", json!({})),
        (
            "launch",
            json!({"app_id": "com.pkg"}),
            "launch",
            json!({"bundle_id": "com.pkg"}),
        ),
        (
            "snapshot",
            json!({"app": "com.pkg"}),
            "snapshot",
            json!({"app": "com.pkg"}),
        ),
    ];
    for (name, arguments, route, body) in cases {
        let request = mcp(name, &arguments);
        assert_eq!(request.as_ref().map(|r| r.verb), Some(route), "{name}");
        assert_eq!(request.map(|r| r.body), Some(body), "{name}");
    }
}

#[test]
fn hold_budget_scales_with_the_press() {
    let request = mcp("hold", &json!({"target": {"ref": "@s:e1"}, "duration": 3}));
    assert_eq!(
        request.and_then(|r| r.timeout),
        Some(Duration::from_secs(33))
    );
}

#[test]
fn type_rejects_an_invalid_explicit_ref() {
    let result = prepare(&json!({"name": "type", "arguments": {"ref": "bad", "text": "hello"}}));
    assert!(matches!(result, Err(Rejected::Tool(f)) if f.field == "ref"));
}

#[test]
fn unknown_tools_and_cli_only_verbs_are_protocol_errors() {
    for name in ["nope", "serve", "mcp"] {
        let result = prepare(&json!({"name": name}));
        assert!(matches!(result, Err(Rejected::Protocol(_))), "{name}");
    }
}

#[test]
fn tool_list_comes_from_the_registry() {
    let all = tools();
    let names: Vec<&str> = all["tools"]
        .as_array()
        .map(|t| t.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    let both: Vec<&str> = VERBS
        .iter()
        .filter(|v| v.surface == Surface::Both)
        .map(|v| v.name)
        .collect();
    assert_eq!(names, both);
    for tool in all["tools"].as_array().into_iter().flatten() {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        for combinator in ["oneOf", "anyOf", "allOf"] {
            assert!(schema.get(combinator).is_none(), "{}", tool["name"]);
        }
    }
}

#[test]
fn devices_filters_are_shared() -> Result<(), std::io::Error> {
    let device = find("devices").ok_or_else(|| std::io::Error::other("missing devices verb"))?;
    let input = serde_json::Map::from_iter([
        ("platform".to_owned(), json!("android")),
        ("kind".to_owned(), json!("emulator")),
        ("state".to_owned(), json!("offline")),
    ]);
    let checked = super::kind::check_args(device, &input, false);
    assert!(checked.is_ok());
    if let Ok(checked) = checked {
        assert_eq!(checked["platform"], "android");
        assert_eq!(checked["kind"], "emulator");
    }
    let invalid = serde_json::Map::from_iter([("platform".to_owned(), json!("windows"))]);
    assert!(super::kind::check_args(device, &invalid, false).is_err());
    Ok(())
}

#[test]
fn devices_output_schema_matches_json_rows() -> Result<(), std::io::Error> {
    let schema =
        listed("devices").ok_or_else(|| std::io::Error::other("missing MCP devices tool"))?;
    for name in ["platform", "kind", "state"] {
        assert!(schema["inputSchema"]["properties"].get(name).is_some());
    }
    let output = &schema["outputSchema"]["properties"]["devices"]["items"]["properties"];
    for name in ["key", "name", "platform", "kind", "os", "state", "serving"] {
        assert!(output.get(name).is_some(), "missing {name}");
    }
    for name in ["os", "state", "serving"] {
        assert_eq!(output[name]["type"], "string");
    }
    let required = &schema["outputSchema"]["properties"]["devices"]["items"]["required"];
    assert_eq!(required, &json!(["key", "name", "platform", "kind"]));
    assert!(output.get("id").is_none());
    assert!(output.get("udid").is_none());
    Ok(())
}

#[test]
fn point_targets_are_a_nested_one_of() {
    let tap = listed("tap");
    let shapes = tap.as_ref().and_then(|t| {
        t["inputSchema"]["properties"]["target"]["oneOf"]
            .as_array()
            .map(Vec::len)
    });
    assert_eq!(shapes, Some(2));
}

#[test]
fn data_replies_declare_output_schemas_and_images_do_not() {
    let declares = |name: &str| listed(name).map(|t| t.get("outputSchema").is_some());
    for name in ["devices", "status", "snapshot", "tap", "stop"] {
        assert_eq!(declares(name), Some(true), "{name}");
    }
    for name in ["screenshot", "skills"] {
        assert_eq!(declares(name), Some(false), "{name}");
    }
    assert!(find("launch").is_some_and(|v| !v.hints.read_only));
}
