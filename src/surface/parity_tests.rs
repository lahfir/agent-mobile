//! Cross-surface execution parity and MCP catalog contract tests.

use serde_json::{Value, json};
use std::ffi::OsString;

use super::mcp;
use super::{Exec, Surface, VERBS, wire_request};
use crate::cmd::Call;
use crate::cmd::reply::Request;

struct Case {
    verb: &'static str,
    argv: &'static [&'static str],
    mcp_arguments: &'static str,
}

const CASES: &[Case] = &[
    Case {
        verb: "devices",
        argv: &["--platform", "android"],
        mcp_arguments: r#"{"platform":"android"}"#,
    },
    Case {
        verb: "status",
        argv: &[],
        mcp_arguments: "{}",
    },
    Case {
        verb: "snapshot",
        argv: &["--app", "com.example.app", "--max-depth", "4"],
        mcp_arguments: r#"{"app":"com.example.app","max_depth":4}"#,
    },
    Case {
        verb: "tap",
        argv: &["@s:e1"],
        mcp_arguments: r#"{"target":{"ref":"@s:e1"}}"#,
    },
    Case {
        verb: "type",
        argv: &["@s:e1", "hi"],
        mcp_arguments: r#"{"ref":"@s:e1","text":"hi"}"#,
    },
    Case {
        verb: "swipe",
        argv: &["down", "@s:e1"],
        mcp_arguments: r#"{"direction":"down","ref":"@s:e1"}"#,
    },
    Case {
        verb: "scroll_until_visible",
        argv: &[
            "Continue",
            "down",
            "--app",
            "com.example.app",
            "--max-depth",
            "5",
        ],
        mcp_arguments: r#"{"text":"Continue","direction":"down","app":"com.example.app","max_depth":5}"#,
    },
    Case {
        verb: "home",
        argv: &[],
        mcp_arguments: "{}",
    },
    Case {
        verb: "doubletap",
        argv: &["@s:e1"],
        mcp_arguments: r#"{"target":{"ref":"@s:e1"}}"#,
    },
    Case {
        verb: "pinch",
        argv: &["@s:e1", "2", "--velocity", "3"],
        mcp_arguments: r#"{"ref":"@s:e1","scale":2,"velocity":3}"#,
    },
    Case {
        verb: "hold",
        argv: &["@s:e1", "--duration", "2"],
        mcp_arguments: r#"{"target":{"ref":"@s:e1"},"duration":2}"#,
    },
    Case {
        verb: "back",
        argv: &[],
        mcp_arguments: "{}",
    },
    Case {
        verb: "twofinger",
        argv: &["@s:e1"],
        mcp_arguments: r#"{"ref":"@s:e1"}"#,
    },
    Case {
        verb: "center",
        argv: &["notification"],
        mcp_arguments: r#"{"which":"notification"}"#,
    },
    Case {
        verb: "launch",
        argv: &["com.example.app"],
        mcp_arguments: r#"{"app_id":"com.example.app"}"#,
    },
    Case {
        verb: "screenshot",
        argv: &["--device", "android:emulator-5554"],
        mcp_arguments: r#"{"device":"android:emulator-5554"}"#,
    },
    Case {
        verb: "stop",
        argv: &["--device", "android:emulator-5554"],
        mcp_arguments: r#"{"device":"android:emulator-5554"}"#,
    },
    Case {
        verb: "release",
        argv: &["--device", "android:emulator-5554"],
        mcp_arguments: r#"{"device":"android:emulator-5554"}"#,
    },
    Case {
        verb: "skills",
        argv: &[],
        mcp_arguments: "{}",
    },
];

fn cli_call(case: &Case) -> Option<Call> {
    let mut argv = vec![OsString::from("agent-mobile"), OsString::from(case.verb)];
    argv.extend(case.argv.iter().map(OsString::from));
    super::cli::parse_from(argv).0.ok()
}

fn mcp_call(case: &Case) -> Option<Call> {
    let arguments: Value = serde_json::from_str(case.mcp_arguments).ok()?;
    let params = json!({"name": case.verb, "arguments": arguments});
    mcp::prepare(&params).ok()
}

fn request(call: &Call) -> Option<Request> {
    wire_request(call.verb, &call.args)
}

fn assert_same_request(cli: &Call, mcp: &Call, name: &str) {
    let cli_request = request(cli);
    let mcp_request = request(mcp);
    assert!(
        cli_request.is_some() && mcp_request.is_some(),
        "missing wire request for {name}"
    );
    if let (Some(cli_request), Some(mcp_request)) = (cli_request, mcp_request) {
        assert_eq!(
            cli_request.verb, mcp_request.verb,
            "route differs for {name}"
        );
        assert_eq!(
            cli_request.body, mcp_request.body,
            "body differs for {name}"
        );
        assert_eq!(
            cli_request.timeout, mcp_request.timeout,
            "timeout differs for {name}"
        );
    }
}

fn assert_same_executor(cli: &Call, mcp: &Call, name: &str) {
    if let (Exec::Local(cli_run), Exec::Local(mcp_run)) = (cli.verb.exec, mcp.verb.exec) {
        assert!(
            std::ptr::fn_addr_eq(cli_run, mcp_run),
            "executor differs for {name}"
        );
    }
}

fn assert_same_execution(cli: &Call, mcp: &Call, name: &str) {
    assert_eq!(
        matches!(cli.verb.exec, Exec::Wire { .. }),
        matches!(mcp.verb.exec, Exec::Wire { .. }),
        "execution kind differs for {name}"
    );
    if matches!(cli.verb.exec, Exec::Wire { .. }) {
        assert_same_request(cli, mcp, name);
    } else {
        assert_same_executor(cli, mcp, name);
    }
}

#[test]
fn every_both_surface_verb_has_an_equivalence_case() {
    for verb in VERBS.iter().filter(|verb| verb.surface == Surface::Both) {
        assert!(
            CASES.iter().any(|case| case.verb == verb.name),
            "missing parity case for {}",
            verb.name
        );
    }
}

#[test]
fn both_surfaces_produce_the_same_execution_input() {
    for case in CASES {
        let cli = cli_call(case);
        let mcp = mcp_call(case);
        assert!(cli.is_some(), "CLI parse failed for {}", case.verb);
        assert!(mcp.is_some(), "MCP parse failed for {}", case.verb);
        let (Some(cli), Some(mcp)) = (cli, mcp) else {
            continue;
        };
        assert_eq!(cli.args, mcp.args, "arguments differ for {}", case.verb);
        assert_same_execution(&cli, &mcp, case.verb);
    }
}

#[test]
fn type_reference_syntax_and_literal_ref_shaped_text_are_distinct() {
    let case = CASES.iter().find(|case| case.verb == "type");
    assert!(case.is_some(), "missing type parity case");
    if let Some(case) = case {
        let call = mcp_call(case);
        assert!(call.is_some());
        if let Some(call) = call {
            assert_eq!(
                request(&call).map(|request| request.body),
                Some(json!({"ref": "@s:e1", "text": "hi"}))
            );
        }
    }
    let literal = mcp::prepare(&json!({"name": "type", "arguments": {"text": "@s:e1"}}));
    assert!(
        matches!(literal, Ok(call) if request(&call).is_some_and(|request| request.body == json!({"text": "@s:e1"})))
    );
}

#[test]
fn devices_platform_filter_matches_between_surfaces() {
    let case = CASES.iter().find(|case| case.verb == "devices");
    assert!(case.is_some(), "missing devices parity case");
    if let Some(case) = case {
        let cli = cli_call(case);
        let mcp = mcp_call(case);
        assert!(
            matches!((&cli, &mcp), (Some(cli), Some(mcp)) if cli.args == mcp.args && cli.args.get("platform") == Some(&json!("android")))
        );
    }
}

#[test]
fn tool_list_matches_reviewed_fixture_and_exposes_release_only() -> Result<(), serde_json::Error> {
    let actual = mcp::tools();
    let generated = serde_json::to_string_pretty(&actual)?;
    let expected: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/tools-list.json"))?;
    assert_eq!(
        actual, expected,
        "Review the change and regenerate tests/fixtures/tools-list.json. Generated JSON:\n{generated}"
    );
    let names: Vec<&str> = actual["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"release"));
    assert!(!names.contains(&"serve"));
    assert!(!names.contains(&"mcp"));
    Ok(())
}
