//! `devices`: list every reachable simulator, device, and AVD across both
//! platforms, marking any with a live session in the state store.

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{State, StateStore};

use crate::platform::{PlatformDevice, discover};

/// Run `devices`; pure discovery, never touches the wire.
pub fn run(json: bool) -> Result<i32, Failure> {
    let scan = discover()?;
    for note in &scan.notes {
        eprintln!("note: {note}");
    }
    let state = StateStore::new().map(|s| s.load()).unwrap_or_default();
    if json {
        let out: Vec<serde_json::Value> = scan
            .devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name(),
                    "platform": d.platform().as_str(),
                    "id": d.id(),
                    "kind": d.kind(),
                    "os": d.os(),
                    "state": d.state(),
                    "serving": serving_url(&state, d),
                })
            })
            .collect();
        let line = serde_json::to_string(&serde_json::json!({ "devices": out }))
            .map_err(|e| Failure::local(e.to_string(), "report a bug"))?;
        super::emit(&line);
        return Ok(0);
    }
    let opt = |v: Option<String>| v.map_or_else(String::new, |v| format!(" {v}"));
    for d in &scan.devices {
        let os = opt(d.os().map(|v| format!("os=\"{v}\"")));
        let st = opt(d.state().map(|v| format!("state={v}")));
        let serving = opt(serving_url(&state, d).map(|u| format!("serving={u}")));
        super::emit(&format!(
            "name=\"{}\" platform={} id={} kind={}{}{}{}",
            d.name(),
            d.platform().as_str(),
            d.id(),
            d.kind(),
            os,
            st,
            serving
        ));
    }
    Ok(0)
}

/// The live session URL serving `device`: the collision-free key first,
/// then the iOS legacy name row — a recorded pid must still be alive.
pub(crate) fn serving_url<'a>(state: &'a State, device: &PlatformDevice) -> Option<&'a str> {
    let live = |key: &str| {
        state
            .devices
            .get(key)
            .filter(|e| agent_mobile_core::process::pid_alive(e.pid))
            .map(|e| e.url.as_str())
    };
    live(&device.key()).or_else(|| device.legacy_key().and_then(live))
}
