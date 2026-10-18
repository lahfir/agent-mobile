//! MCP per-call device selection and R9 conflict behavior.

#[allow(dead_code, reason = "shared CLI and driver fixtures")]
mod common;
#[path = "common/mcp_client.rs"]
#[allow(dead_code, reason = "shared MCP client constructors")]
mod mcp_client;

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{SessionEntry, StateStore};
use mcp_client::Client;
use serde_json::json;

#[test]
fn per_call_device_reports_the_live_device_without_changing_default() -> Result<(), Failure> {
    let home = common::tmp_home("mcp-selection")?;
    let root = home.join(".agent-mobile");
    std::fs::create_dir_all(&root)?;
    let store = StateStore::at(&root);
    let mut entry = SessionEntry::new(
        "http://127.0.0.1:1".to_owned(),
        std::process::id(),
        "fixture".to_owned(),
    );
    entry.process_started_at = agent_mobile_core::process::process_identity(std::process::id());
    store.upsert("ios:A", &entry)?;
    store.remember_device_selection("ios:A", "A")?;
    let mut client = Client::with_args(&["--device", "ios:A", "--json", "mcp"], &home, &[])?;
    client.legacy()?;
    let reply = client.request(
        1,
        "tools/call",
        &json!({
            "name":"snapshot",
            "arguments":{"device":"android:avd:x"}
        }),
    )?;
    assert_eq!(reply["result"]["isError"], true);
    let message = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("ios:A is being served"), "{message}");
    assert!(message.contains("release"), "{message}");
    assert_eq!(store.load().default_device_key.as_deref(), Some("ios:A"));
    client.finish()
}
