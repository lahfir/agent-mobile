//! Device discovery (KTD5): `devices` lists reachable simulators and paired
//! physical devices by shelling out to `simctl` and `devicectl`. The physical
//! half is best-effort — a simulator list still ships when devicectl fails —
//! while a broken `simctl` means Xcode itself is missing, which is fatal with
//! the install step named. Driver launch lives in [`runner`].

mod runner;

pub use runner::{
    DriverSource, boot_simulator, driver_addr, driver_source, driver_url, serve_command,
};

use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::error::Failure;

/// Default driver port; one port serves one device.
pub const DEFAULT_PORT: u16 = 8770;

/// Bound on any one `xcrun` probe — a wedged `CoreSimulatorService` or
/// devicectl must not hang a verb forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

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

/// `list_devices` output: the devices found plus best-effort probe
/// failures a caller may surface — a devicectl error means paired iPhones
/// went unseen rather than proven absent.
#[derive(Debug)]
pub struct DeviceScan {
    /// Reachable devices.
    pub devices: Vec<Device>,
    /// Non-fatal probe failures worth telling the operator about.
    pub notes: Vec<String>,
}

/// List reachable simulators plus paired physical devices; `simctl` and
/// `devicectl` probe in parallel since either can take seconds.
///
/// # Errors
/// Returns [`Failure::Local`] when `xcrun simctl` cannot run or its output
/// cannot be parsed — the Xcode install is broken or absent.
pub fn list_devices() -> Result<DeviceScan, Failure> {
    std::thread::scope(|s| {
        let sims = s.spawn(simulators);
        let phys = s.spawn(physical_probe);
        let mut devices = sims
            .join()
            .map_err(|_| Failure::local("simctl scan panicked", "report a bug"))??;
        let mut notes = Vec::new();
        match phys.join() {
            Ok(Ok(found)) => devices.extend(found),
            Ok(Err(reason)) => notes.push(reason),
            Err(_) => notes.push("devicectl probe panicked".to_owned()),
        }
        Ok(DeviceScan { devices, notes })
    })
}

/// Available simulators only; skips the slower `devicectl` probe.
///
/// # Errors
/// Returns [`Failure::Local`] when `xcrun simctl` cannot run or its output
/// cannot be parsed — the Xcode install is broken or absent.
pub fn simulators() -> Result<Vec<Device>, Failure> {
    let mut cmd = Command::new("xcrun");
    cmd.args(["simctl", "list", "devices", "available", "--json"]);
    let out =
        crate::process::run_bounded(&mut cmd, PROBE_TIMEOUT).map_err(|e| simctl_spawn_err(&e))?;
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

/// Paired physical devices only; best-effort — `devicectl` failures return
/// an empty list so callers that just want a device never see them.
#[must_use]
pub fn physical() -> Vec<Device> {
    physical_probe().unwrap_or_default()
}

/// The verbose form of [`physical`]: `Err` carries why `devicectl` produced
/// nothing, so `devices` can distinguish "no iPhone paired" from "probe
/// failed".
///
/// # Errors
/// Returns a human-readable reason when `devicectl` fails to run, times out,
/// exits nonzero, or answers unparseable JSON.
pub fn physical_probe() -> Result<Vec<Device>, String> {
    let mut cmd = Command::new("xcrun");
    cmd.args([
        "devicectl",
        "list",
        "devices",
        "--timeout",
        "5",
        "--json-output",
        "-",
    ]);
    let out = crate::process::run_bounded(&mut cmd, PROBE_TIMEOUT)
        .map_err(|e| format!("devicectl probe failed: {}", e.message()))?;
    if !out.status.success() {
        return Err(format!(
            "devicectl exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("cannot parse devicectl output: {e}"))?;
    let mut devices = Vec::new();
    for d in v
        .pointer("/result/devices")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let props = d.get("deviceProperties").unwrap_or(&Value::Null);
        let conn = d.get("connectionProperties").unwrap_or(&Value::Null);
        let name = field(props, "name").trim().to_owned();
        let udid = d
            .pointer("/hardwareProperties/udid")
            .and_then(Value::as_str)
            .map_or_else(|| field(d, "identifier"), str::to_owned);
        if name.is_empty() || udid.is_empty() {
            continue;
        }
        devices.push(Device {
            name,
            udid,
            kind: "device",
            os: props
                .get("osVersionNumber")
                .and_then(Value::as_str)
                .map(str::to_owned),
            state: conn
                .get("tunnelState")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    Ok(devices)
}

fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-26-0` -> `26.0` — the bare
/// version `status` reports, keeping `devices` and `status` consistent.
/// Non-iOS families keep their prefix so they stay identifiable.
fn runtime_label(runtime: &str) -> Option<String> {
    let tail = runtime.strip_prefix("com.apple.CoreSimulator.SimRuntime.")?;
    let (family, ver) = tail.split_once('-')?;
    let ver = ver.replace('-', ".");
    if family == "iOS" {
        Some(ver)
    } else {
        Some(format!("{family} {ver}"))
    }
}

/// Find one device by name or udid. Simulators match first — same result
/// as scanning the combined list, and `devicectl` only runs on a miss.
///
/// # Errors
/// Returns [`Failure::Local`] when discovery itself fails.
pub fn find_device(name_or_udid: &str) -> Result<Option<Device>, Failure> {
    let hit = |d: &Device| d.name == name_or_udid || d.udid.eq_ignore_ascii_case(name_or_udid);
    if let Some(d) = simulators()?.into_iter().find(hit) {
        return Ok(Some(d));
    }
    Ok(physical().into_iter().find(hit))
}

/// An iPhone's Bonjour host name: apostrophes drop, spaces become hyphens —
/// `Lahfir's iPhone` becomes `Lahfirs-iPhone.local`.
#[must_use]
pub fn bonjour_host(device_name: &str) -> String {
    let mut out = String::new();
    for c in device_name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if c == ' ' || c == '-' {
            out.push('-');
        }
    }
    format!("{out}.local")
}

/// The probe failure every `simctl` call maps the same way: a spawn or
/// timeout blames the probe itself, with the install fix named.
pub(super) fn simctl_spawn_err(e: &Failure) -> Failure {
    let detail = match e {
        Failure::Local { message, .. } => message.clone(),
        _ => e.render(),
    };
    let next = if detail.contains("did not answer") {
        "run `xcrun simctl list devices` yourself to see what it is waiting on"
    } else {
        "install Xcode, then run `sudo xcodebuild -license accept`"
    };
    Failure::local(format!("`xcrun simctl` failed: {detail}"), next)
}
