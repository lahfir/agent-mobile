//! Device discovery (KTD5): `devices` lists reachable simulators and paired
//! physical devices by shelling out to `simctl` and `devicectl`. The physical
//! half is best-effort — a simulator list still ships when devicectl fails —
//! while a broken `simctl` means Xcode itself is missing, which is fatal with
//! the install step named.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

use crate::error::Failure;

/// Default driver port; one port serves one device.
pub const DEFAULT_PORT: u16 = 8770;

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

/// Find one device by name or udid.
///
/// # Errors
/// Returns [`Failure::Local`] when discovery itself fails.
pub fn find_device(name_or_udid: &str) -> Result<Option<Device>, Failure> {
    let devices = list_devices()?;
    Ok(devices
        .into_iter()
        .find(|d| d.name == name_or_udid || d.udid.eq_ignore_ascii_case(name_or_udid)))
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

/// Boot a shutdown simulator; "already booted" is success.
///
/// # Errors
/// Returns [`Failure::Local`] on a real boot failure, naming the simctl
/// create command when the device is missing.
pub fn boot_simulator(udid: &str) -> Result<(), Failure> {
    let out = Command::new("xcrun")
        .args(["simctl", "boot", udid])
        .output()
        .map_err(|e| {
            Failure::local(
                format!("cannot run `xcrun simctl`: {e}"),
                "install Xcode, then run `sudo xcodebuild -license accept`",
            )
        })?;
    let err = String::from_utf8_lossy(&out.stderr);
    if out.status.success() || err.contains("current state: Booted") {
        return Ok(());
    }
    if err.contains("does not exist") || err.contains("Invalid device") {
        return Err(Failure::local(
            format!("no simulator {udid}"),
            "create one with `xcrun simctl create <name> <device-type>`",
        ));
    }
    Err(Failure::local(
        format!("simctl boot failed: {}", err.trim()),
        "open Simulator.app once, then retry",
    ))
}

/// The directory holding the driver's Xcode project. `AGENT_MOBILE_DRIVER_DIR`
/// wins for the packaged layout; otherwise walk up from the core crate's
/// manifest dir — `crates/core` — to the checkout root's `fixtures/driver`.
///
/// # Errors
/// Returns [`Failure::Local`] when no project exists at any probed path.
pub fn driver_dir() -> Result<PathBuf, Failure> {
    let mut probes: Vec<PathBuf> = std::env::var_os("AGENT_MOBILE_DRIVER_DIR")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    probes.extend(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|a| a.join("fixtures/driver")),
    );
    probes
        .into_iter()
        .find(|dir| dir.join("ToDo.xcodeproj").exists())
        .ok_or_else(|| {
            Failure::local(
                "no driver project found near this binary",
                "set AGENT_MOBILE_DRIVER_DIR to the bundled runner directory",
            )
        })
}

/// Build the `xcodebuild test` invocation that runs the driver on `device`.
/// Environment goes through `TEST_RUNNER_`-prefixed variables — the only
/// channel xcodebuild forwards into the test runner.
#[must_use]
pub fn serve_command(device: &Device, port: u16, token: &str, driver_dir: &PathBuf) -> Command {
    let mut cmd = Command::new("xcodebuild");
    let (destination, dd, extra): (String, &str, &[&str]) = if device.kind == "simulator" {
        (
            format!("platform=iOS Simulator,id={}", device.udid),
            "dd",
            &["CODE_SIGNING_ALLOWED=NO"][..],
        )
    } else {
        (
            format!("platform=iOS,id={}", device.udid),
            "dd-device",
            &["-allowProvisioningUpdates"][..],
        )
    };
    cmd.current_dir(driver_dir)
        .args([
            "test",
            "-project",
            "ToDo.xcodeproj",
            "-scheme",
            "ToDo",
            "-destination",
            &destination,
            "-only-testing:ToDoUITests/AgentMobileServer/testServe",
            "-skip-testing:ToDoTests",
            "-parallel-testing-enabled",
            "NO",
            "-derivedDataPath",
            dd,
        ])
        .args(extra.iter().copied())
        .env("TEST_RUNNER_AGENT_MOBILE_PORT", port.to_string())
        .env("TEST_RUNNER_AGENT_MOBILE_TOKEN", token);
    if device.kind != "simulator" {
        cmd.env("TEST_RUNNER_AGENT_MOBILE_BIND", "0.0.0.0");
    }
    cmd
}

/// The URL a running driver answers on: loopback for a simulator, the
/// device's Bonjour host for a physical iPhone.
#[must_use]
pub fn driver_url(device: &Device, port: u16) -> String {
    if device.kind == "simulator" {
        format!("http://127.0.0.1:{port}")
    } else {
        format!("http://{}:{port}", bonjour_host(&device.name))
    }
}
