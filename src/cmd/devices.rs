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
        let out: Vec<serde_json::Value> =
            scan.devices.iter().map(|d| row_json(&state, d)).collect();
        let line = serde_json::to_string(&serde_json::json!({ "devices": out }))
            .map_err(|e| Failure::local(e.to_string(), "report a bug"))?;
        super::emit(&line);
        return Ok(0);
    }
    for d in &scan.devices {
        super::emit(&row_text(&state, d));
    }
    Ok(0)
}

/// One device's JSON row: `udid` is the deprecated alias for `id` kept so
/// mixed-platform consumers iterating fields cannot `KeyError`.
fn row_json(state: &State, d: &PlatformDevice) -> serde_json::Value {
    serde_json::json!({
        "name": d.name(),
        "platform": d.platform().as_str(),
        "key": d.key(),
        "id": d.id(),
        "udid": d.id(),
        "kind": d.kind(),
        "os": d.os(),
        "state": d.state(),
        "serving": serving_url(state, d),
    })
}

/// One device's text row — `name`/`udid` lead exactly as the original
/// wire shape did, new fields appended after.
fn row_text(state: &State, d: &PlatformDevice) -> String {
    let opt = |v: Option<String>| v.map_or_else(String::new, |v| format!(" {v}"));
    let os = opt(d.os().map(|v| format!("os=\"{v}\"")));
    let st = opt(d.state().map(|v| format!("state={v}")));
    let serving = opt(serving_url(state, d).map(|u| format!("serving={u}")));
    format!(
        "name=\"{}\" udid={} platform={} key={} id={} kind={}{}{}{}",
        d.name(),
        d.id(),
        d.platform().as_str(),
        d.key(),
        d.id(),
        d.kind(),
        os,
        st,
        serving
    )
}

/// The live session URL serving `device`: the collision-free key first,
/// then the iOS legacy name row — a recorded pid must still be alive.
pub(crate) fn serving_url<'a>(state: &'a State, device: &PlatformDevice) -> Option<&'a str> {
    let live = |key: &str| {
        state
            .devices
            .get(key)
            .filter(|e| {
                agent_mobile_core::process::process_matches(e.pid, e.process_started_at.as_deref())
            })
            .map(|e| e.url.as_str())
    };
    live(&device.key()).or_else(|| device.legacy_key().and_then(live))
}

#[cfg(test)]
mod row_tests {
    use super::*;
    use crate::platform::PlatformDevice;
    use agent_mobile_android::{AndroidDeviceKind, AndroidDeviceState, AndroidTarget};

    fn ios() -> PlatformDevice {
        PlatformDevice::from_ios(agent_mobile_core::ios::Device {
            name: "iPhone 17".to_owned(),
            udid: "UDID-1".to_owned(),
            kind: "simulator",
            os: Some("26.0".to_owned()),
            state: Some("Booted".to_owned()),
        })
    }

    fn android() -> PlatformDevice {
        PlatformDevice::from_android(AndroidTarget {
            id: "avd:api37".to_owned(),
            name: "api37".to_owned(),
            serial: None,
            kind: AndroidDeviceKind::Emulator,
            state: AndroidDeviceState::Other("shutdown".to_owned()),
            model: None,
            product: None,
            os: None,
            avd: None,
        })
    }

    #[test]
    fn rows_carry_udid_alias_key_and_legacy_prefix() {
        let state = State::default();
        for d in [ios(), android()] {
            let row = row_json(&state, &d);
            assert_eq!(row["udid"], row["id"], "udid must alias id");
            assert_eq!(row["key"], format!("{}:{}", d.platform().as_str(), d.id()));
            let text = row_text(&state, &d);
            assert!(
                text.starts_with(&format!(
                    "name=\"{}\" udid={} platform={}",
                    d.name(),
                    d.id(),
                    d.platform().as_str()
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!(" key={} id={} kind={}", d.key(), d.id(), d.kind())),
                "{text}"
            );
        }
    }
}
