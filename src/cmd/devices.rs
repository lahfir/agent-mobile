//! `devices`: list reachable simulators and paired devices, marking any with
//! a live session in the state store.

use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;
use agent_mobile_core::state::StateStore;

/// Run `devices`; pure discovery, never touches the wire.
pub fn run(json: bool) -> Result<i32, Failure> {
    let devices = ios::list_devices()?;
    let state = StateStore::new().map(|s| s.load()).unwrap_or_default();
    if json {
        let out: Vec<serde_json::Value> = devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "udid": d.udid,
                    "kind": d.kind,
                    "os": d.os,
                    "state": d.state,
                    "serving": state.devices.get(&d.name).map(|e| &e.url),
                })
            })
            .collect();
        let line = serde_json::to_string(&serde_json::json!({ "devices": out }))
            .map_err(|e| Failure::local(e.to_string(), "report a bug"))?;
        println!("{line}");
        return Ok(0);
    }
    for d in &devices {
        let os =
            d.os.as_ref()
                .map_or_else(String::new, |v| format!(" os=\"{v}\""));
        let st = d
            .state
            .as_ref()
            .map_or_else(String::new, |v| format!(" state={v}"));
        let serving = state
            .devices
            .get(&d.name)
            .map_or_else(String::new, |e| format!(" serving={}", e.url));
        println!(
            "name=\"{}\" udid={} kind={}{}{}{}",
            d.name, d.udid, d.kind, os, st, serving
        );
    }
    Ok(0)
}
