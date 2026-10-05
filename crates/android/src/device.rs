//! Device discovery and AVD boot: `adb devices -l` parsing retains every
//! state row, running emulators correlate to configured AVDs via
//! `emu avd name`, and `boot_avd` spawns detached so the emulator outlives
//! this process.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use agent_mobile_core::error::Failure;

use crate::adb::Adb;

/// How a discovered target is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AndroidDeviceKind {
    /// `emulator-5554`-style serial.
    Emulator,
    /// USB-attached physical device.
    Usb,
    /// `host:port` TCP device.
    Wireless,
}

/// `adb devices` state, keeping unknown values rather than dropping rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AndroidDeviceState {
    /// Ready for `adb` commands.
    Device,
    /// Transport saw the device but it is not answering.
    Offline,
    /// Present but not yet authorized for this host.
    Unauthorized,
    /// Any other reported state.
    Other(String),
}

/// One reachable or configured Android target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidTarget {
    /// `avd:<name>` for AVDs (running or stopped), else the serial.
    pub id: String,
    /// Display name — AVD name, device model, or serial.
    pub name: String,
    /// `adb` serial when the target is reachable now.
    pub serial: Option<String>,
    /// Attachment kind.
    pub kind: AndroidDeviceKind,
    /// Last reported `adb` state.
    pub state: AndroidDeviceState,
    /// `model:` descriptor when reported.
    pub model: Option<String>,
    /// `product:` descriptor when reported.
    pub product: Option<String>,
    /// `ro.build.version.release` for live targets; `None` otherwise.
    pub os: Option<String>,
    /// Configured AVD name this target represents.
    pub avd: Option<String>,
}

/// Discovery result: targets plus non-fatal probe notes.
#[derive(Debug)]
pub struct AndroidScan {
    /// Devices and configured AVDs.
    pub targets: Vec<AndroidTarget>,
    /// Non-fatal probe failures worth surfacing.
    pub notes: Vec<String>,
}

/// A freshly booted (or already-running) AVD. Dropping this never kills the
/// emulator — it owns its own lifecycle.
#[derive(Debug)]
pub struct BootedAvd {
    /// `adb` serial the emulator answers on.
    pub serial: String,
    /// Spawned emulator pid; `0` when the AVD was already running.
    pub pid: u32,
    /// Private log file the emulator output was appended to.
    pub log: PathBuf,
}

/// One parsed `devices -l` row.
#[derive(Debug)]
pub(crate) struct DeviceRow {
    /// First column: the serial.
    pub(crate) serial: String,
    /// Second column: the state word.
    pub(crate) state: String,
    /// `key:value` descriptor fields.
    pub(crate) fields: BTreeMap<String, String>,
}

/// Parse `adb devices -l` output into rows, skipping the header and blanks.
#[must_use]
pub(crate) fn parse_devices(output: &str) -> Vec<DeviceRow> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            if serial == "List" {
                return None;
            }
            let state = parts.next()?;
            let fields = parts
                .filter_map(|tok| tok.split_once(':'))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            Some(DeviceRow {
                serial: serial.to_owned(),
                state: state.to_owned(),
                fields,
            })
        })
        .collect()
}

/// Classify a serial into emulator/USB/wireless.
pub(crate) fn kind_of(serial: &str) -> AndroidDeviceKind {
    if serial.starts_with("emulator-") {
        AndroidDeviceKind::Emulator
    } else if serial.contains(':') {
        AndroidDeviceKind::Wireless
    } else {
        AndroidDeviceKind::Usb
    }
}

/// Map the state word, preserving unknown values.
fn state_of(state: &str) -> AndroidDeviceState {
    match state {
        "device" => AndroidDeviceState::Device,
        "offline" => AndroidDeviceState::Offline,
        "unauthorized" => AndroidDeviceState::Unauthorized,
        other => AndroidDeviceState::Other(other.to_owned()),
    }
}

/// `adb -s <serial> emu avd name` — the console answer ends with `OK`.
///
/// # Errors
/// Propagates runner failures; a refused console query yields `None`.
pub(crate) fn avd_name(adb: &Adb, serial: &str) -> Result<Option<String>, Failure> {
    let out = adb.scoped(serial, &["emu", "avd", "name"])?;
    if !out.success {
        return Ok(None);
    }
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && *l != "OK")
        .map(str::to_owned))
}

/// Configured AVD names from `emulator -list-avds`.
///
/// # Errors
/// Runner/parse failures surface as [`Failure::Local`].
pub(crate) fn list_avds(adb: &Adb, emulator: &Path) -> Result<Vec<String>, Failure> {
    let out = adb.tool(emulator, &["-list-avds"])?;
    if !out.success {
        return Err(Failure::local(
            format!("emulator -list-avds failed: {}", out.stderr),
            "check the Android SDK emulator package",
        ));
    }
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Full scan: `devices -l` rows plus configured AVDs, correlating running
/// emulators to their AVD names.
///
/// # Errors
/// [`Failure::Local`] when `adb devices` itself cannot run.
pub(crate) fn discover(adb: &Adb, emulator: &Path) -> Result<AndroidScan, Failure> {
    let out = adb.unscoped(&["devices", "-l"])?;
    if !out.success {
        return Err(Failure::local(
            format!("adb devices failed: {}", out.stderr),
            "restart the adb server and retry",
        ));
    }
    let mut notes = Vec::new();
    let configured = match list_avds(adb, emulator) {
        Ok(avds) => avds,
        Err(e) => {
            notes.push(format!("emulator -list-avds: {}", e.message()));
            Vec::new()
        }
    };
    let mut targets = Vec::new();
    let mut running_avds = HashSet::new();
    for row in parse_devices(&out.stdout) {
        targets.push(target_for(adb, &row, &configured, &mut running_avds));
    }
    for name in configured {
        if !running_avds.contains(&name) {
            targets.push(cold_avd(&name));
        }
    }
    for t in &mut targets {
        if t.state == AndroidDeviceState::Device
            && let Some(serial) = t.serial.as_deref()
        {
            t.os = probe_os(adb, serial);
        }
    }
    Ok(AndroidScan { targets, notes })
}

/// Turn one device row into a target, correlating running emulators.
fn target_for(
    adb: &Adb,
    row: &DeviceRow,
    configured: &[String],
    running: &mut HashSet<String>,
) -> AndroidTarget {
    let kind = kind_of(&row.serial);
    let state = state_of(&row.state);
    let mut id = row.serial.clone();
    let mut name = row
        .fields
        .get("model")
        .cloned()
        .unwrap_or_else(|| row.serial.clone());
    let mut avd = None;
    if kind == AndroidDeviceKind::Emulator
        && state == AndroidDeviceState::Device
        && let Ok(Some(found)) = avd_name(adb, &row.serial)
        && configured.iter().any(|c| c == &found)
    {
        id = format!("avd:{found}");
        name.clone_from(&found);
        running.insert(found.clone());
        avd = Some(found);
    }
    AndroidTarget {
        id,
        name,
        serial: Some(row.serial.clone()),
        kind,
        state,
        model: row.fields.get("model").cloned(),
        product: row.fields.get("product").cloned(),
        os: None,
        avd,
    }
}

/// `shell getprop ro.build.version.release` for a live serial; a nonzero
/// or empty reply is `None` and never fails the scan.
fn probe_os(adb: &Adb, serial: &str) -> Option<String> {
    let out = adb
        .scoped(serial, &["shell", "getprop", "ro.build.version.release"])
        .ok()?;
    if !out.success {
        return None;
    }
    let v = out.stdout.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_owned())
    }
}

/// A configured-but-stopped AVD target.
fn cold_avd(name: &str) -> AndroidTarget {
    AndroidTarget {
        id: format!("avd:{name}"),
        name: name.to_owned(),
        serial: None,
        kind: AndroidDeviceKind::Emulator,
        state: AndroidDeviceState::Other("shutdown".to_owned()),
        model: None,
        product: None,
        os: None,
        avd: Some(name.to_owned()),
    }
}

#[cfg(test)]
mod tests;
