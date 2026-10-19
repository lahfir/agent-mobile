//! `devices`: list every reachable simulator, device, and AVD across both
//! platforms, marking any with a live session in the state store.

use agent_mobile_core::error::Failure;
use agent_mobile_core::state::{State, StateStore};

use crate::platform::{PlatformDevice, discover_on};

/// The `devices` verb: pure discovery, never touches the wire.
///
/// # Errors
/// Discovery failures on both platforms.
pub fn exec(call: &super::Call) -> Result<super::Reply, Failure> {
    let scan = discover_on(call.str("platform"))?;
    for note in &scan.notes {
        eprintln!("note: {note}");
    }
    let state = StateStore::new().map(|s| s.load()).unwrap_or_default();
    let devices: Vec<_> = scan
        .devices
        .into_iter()
        .filter(|device| {
            matches_filters(
                device,
                call.str("platform"),
                call.str("kind"),
                call.str("state"),
            )
        })
        .collect();
    Ok(reply(&devices, &state))
}

fn matches_filters(
    device: &PlatformDevice,
    platform: Option<&str>,
    kind: Option<&str>,
    state: Option<&str>,
) -> bool {
    platform.is_none_or(|value| device.platform().as_str() == value)
        && kind.is_none_or(|value| device.kind() == value)
        && state
            .is_none_or(|value| device.state().as_deref() == Some(value.to_lowercase().as_str()))
}

/// Device rows in both forms, without writing to stdout.
#[must_use]
pub fn reply(devices: &[PlatformDevice], state: &State) -> super::Reply {
    super::Reply::Devices {
        rows: devices.iter().map(|d| row_json(state, d)).collect(),
        text: devices
            .iter()
            .map(|d| row_text(state, d))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// One device's JSON row, identified by its collision-free key.
fn row_json(state: &State, d: &PlatformDevice) -> serde_json::Value {
    serde_json::json!({
        "name": d.name(),
        "platform": d.platform().as_str(),
        "key": d.key(),
        "kind": d.kind(),
        "os": d.os(),
        "state": d.state(),
        "serving": serving_url(state, d),
    })
}

/// One device's text row, with its collision-free key first.
fn row_text(state: &State, d: &PlatformDevice) -> String {
    let opt = |v: Option<String>| v.map_or_else(String::new, |v| format!(" {v}"));
    let os = opt(d.os().map(|v| format!("os=\"{v}\"")));
    let st = opt(d.state().map(|v| format!("state={v}")));
    let serving = opt(serving_url(state, d).map(|u| format!("serving={u}")));
    format!(
        "key={} name=\"{}\" platform={} kind={}{}{}{}",
        d.key(),
        d.name(),
        d.platform().as_str(),
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
    fn rows_use_key_identity_without_legacy_ids() {
        let state = State::default();
        for d in [ios(), android()] {
            let row = row_json(&state, &d);
            assert_eq!(row["key"], format!("{}:{}", d.platform().as_str(), d.id()));
            assert!(row.get("id").is_none());
            assert!(row.get("udid").is_none());
            let text = row_text(&state, &d);
            assert!(
                text.starts_with(&format!("key={} name=\"{}\"", d.key(), d.name())),
                "{text}"
            );
            assert!(!text.contains(" udid="), "{text}");
            assert!(!text.contains(" id="), "{text}");
        }
    }

    #[test]
    fn filters_combine_and_preserve_lowercase_state_matching() {
        let ios = ios();
        let android = android();
        assert!(matches_filters(
            &ios,
            Some("ios"),
            Some("simulator"),
            Some("BOOTED")
        ));
        assert!(!matches_filters(&ios, Some("android"), None, None));
        assert!(!matches_filters(&ios, None, Some("device"), None));
        assert!(matches_filters(
            &android,
            Some("android"),
            Some("emulator"),
            Some("shutdown")
        ));
    }
}
