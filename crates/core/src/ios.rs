//! Device discovery (KTD5): `devices` lists reachable simulators and paired
//! physical devices by shelling out to `simctl` and `devicectl`. The physical
//! half is best-effort — a simulator list still ships when devicectl fails —
//! while a broken `simctl` means Xcode itself is missing, which is fatal with
//! the install step named.

use std::process::Command;

use serde_json::Value;

use crate::error::Failure;

/// One reachable device or simulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Display name, e.g. `iPhone 17 Pro Max`.
    pub name: String,
    /// Unique identifier: sim UDID or devicectl identifier.
    pub udid: String,
    /// `simulator` or `device`.
    pub kind: &'static str,
    /// OS version when known, e.g. `iOS 26.0`.
    pub os: Option<String>,
    /// Boot state for sims or tunnel state for devices.
    pub state: Option<String>,
}

/// List reachable simulators plus paired physical devices.
///
/// # Errors
/// Returns [`Failure::Local`] when `xcrun simctl` cannot run or its output
/// cannot be parsed — the Xcode install is broken or absent.
pub fn list_devices() -> Result<Vec<Device>, Failure> {
    let mut out = simulators()?;
    out.extend(physical());
    Ok(out)
}

fn simulators() -> Result<Vec<Device>, Failure> {
    let out = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "--json"])
        .output()
        .map_err(|e| {
            Failure::local(
                format!("cannot run `xcrun simctl`: {e}"),
                "install Xcode, then run `sudo xcodebuild -license accept`",
            )
        })?;
    if !out.status.success() {
        return Err(Failure::local(
            format!(
                "`xcrun simctl` failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "fix the Xcode install, then retry",
        ));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        Failure::local(
            format!("cannot parse simctl output: {e}"),
            "report a bug with the simctl JSON attached",
        )
    })?;
    let mut devices = Vec::new();
    let Some(runtimes) = v.get("devices").and_then(Value::as_object) else {
        return Ok(devices);
    };
    for (runtime, list) in runtimes {
        let os = runtime_label(runtime);
        for d in list.as_array().map_or(&[][..], Vec::as_slice) {
            if d.get("isAvailable") != Some(&Value::Bool(true)) {
                continue;
            }
            devices.push(Device {
                name: field(d, "name"),
                udid: field(d, "udid"),
                kind: "simulator",
                os: os.clone(),
                state: Some(field(d, "state")),
            });
        }
    }
    Ok(devices)
}

fn physical() -> Vec<Device> {
    let Ok(out) = Command::new("xcrun")
        .args([
            "devicectl",
            "list",
            "devices",
            "--timeout",
            "5",
            "--json-output",
            "-",
        ])
        .output()
    else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for d in v
        .pointer("/result/devices")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let props = d.get("deviceProperties").cloned().unwrap_or(Value::Null);
        let conn = d
            .get("connectionProperties")
            .cloned()
            .unwrap_or(Value::Null);
        let name = props
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        devices.push(Device {
            name,
            udid: field(d, "identifier"),
            kind: "device",
            os: props
                .get("osVersionNumber")
                .and_then(Value::as_str)
                .map(|v| format!("iOS {v}")),
            state: conn
                .get("tunnelState")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    devices
}

fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn runtime_label(runtime: &str) -> Option<String> {
    let tail = runtime.strip_prefix("com.apple.CoreSimulator.SimRuntime.")?;
    let (family, ver) = tail.split_once('-')?;
    Some(format!("{family} {}", ver.replace('-', ".")))
}
