//! Driver launch (KTD7, KTD14): where the runner comes from and the
//! `xcodebuild` invocation that puts it on a device, plus the boot step a
//! shutdown simulator needs first and the address a running driver binds.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{Device, simctl_spawn_err};
use crate::error::Failure;

/// The single test the xcodebuild invocation runs, whichever source shape
/// the driver came from.
const TEST_ONLY: &str = "-only-testing:AgentMobileDriver/AgentMobileServer/testServe";

/// Boot a shutdown simulator; "already booted" is success.
///
/// # Errors
/// Returns [`Failure::Local`] on a real boot failure, naming the simctl
/// create command when the device is missing.
pub fn boot_simulator(udid: &str) -> Result<(), Failure> {
    let mut cmd = Command::new("xcrun");
    cmd.args(["simctl", "boot", udid]);
    let out = crate::process::run_bounded(&mut cmd, Duration::from_secs(60))
        .map_err(|e| simctl_spawn_err(&e))?;
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

/// The `-destination` string for a simulator target.
fn sim_destination(device: &Device) -> String {
    format!("platform=iOS Simulator,id={}", device.udid)
}

/// Where the runner comes from: a source project to build, or a bundled
/// `.xctestrun` product that needs no compiler (KTD14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverSource {
    /// A checkout's `drivers/ios` holding `AgentMobileDriver.xcodeproj`.
    Project(PathBuf),
    /// A packaged runner dir holding `*.xctestrun` plus its `__TESTROOT__`
    /// products — `test-without-building` installs and runs it directly.
    /// `serve_command` passes the manifest by basename because the child's
    /// cwd is already `dir`; a relative `AGENT_MOBILE_DRIVER_DIR` would
    /// otherwise double the path.
    Prebuilt {
        /// Directory containing the xctestrun manifest and products.
        dir: PathBuf,
        /// The `*.xctestrun` manifest itself.
        xctestrun: PathBuf,
    },
}

/// Resolve the driver source: `AGENT_MOBILE_DRIVER_DIR` wins, then a
/// `runner/` dir beside the executable (the npm layout), then the checkout's
/// `drivers/ios` walked up from the core crate's manifest dir.
///
/// # Errors
/// Returns [`Failure::Local`] when neither a project nor a bundled runner
/// exists at any probed path.
pub fn driver_source() -> Result<DriverSource, Failure> {
    let mut probes: Vec<PathBuf> = std::env::var_os("AGENT_MOBILE_DRIVER_DIR")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        probes.push(dir.join("../runner"));
        probes.push(dir.join("runner"));
    }
    probes.extend(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|a| a.join("drivers/ios")),
    );
    for dir in probes {
        if let Some(src) = classify_driver_dir(&dir) {
            return Ok(src);
        }
    }
    Err(Failure::local(
        "no driver project or bundled runner found",
        "set AGENT_MOBILE_DRIVER_DIR to a driver checkout or a bundled runner directory",
    ))
}

/// A dir holding `AgentMobileDriver.xcodeproj` is a project; one holding
/// `*.xctestrun` is a prebuilt runner. Anything else is not a driver source.
fn classify_driver_dir(dir: &Path) -> Option<DriverSource> {
    if dir.join("AgentMobileDriver.xcodeproj").exists() {
        return Some(DriverSource::Project(dir.to_path_buf()));
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let xctestrun = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "xctestrun"))?;
    Some(DriverSource::Prebuilt {
        dir: dir.to_path_buf(),
        xctestrun,
    })
}

/// Build the `xcodebuild` invocation that runs the driver on `device`:
/// `test` against the project, or `test-without-building` against the
/// bundled runner. Environment goes through `TEST_RUNNER_`-prefixed
/// variables — the only channel xcodebuild forwards into the test runner.
///
/// # Errors
/// Returns [`Failure::Local`] for a physical device with only a bundled
/// runner: device signing is per-Mac and cannot ship prebuilt.
pub fn serve_command(
    device: &Device,
    port: u16,
    token: &str,
    source: &DriverSource,
) -> Result<Command, Failure> {
    let mut cmd = Command::new("xcodebuild");
    match source {
        DriverSource::Project(dir) => {
            let (destination, dd, extra): (String, &str, &[&str]) = if device.kind == "simulator" {
                (
                    sim_destination(device),
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
            cmd.current_dir(dir)
                .args([
                    "test",
                    "-project",
                    "AgentMobileDriver.xcodeproj",
                    "-scheme",
                    "AgentMobileDriver",
                    "-destination",
                    &destination,
                    TEST_ONLY,
                    "-parallel-testing-enabled",
                    "NO",
                    "-derivedDataPath",
                    dd,
                ])
                .args(extra.iter().copied());
        }
        DriverSource::Prebuilt { dir, xctestrun } => {
            if device.kind != "simulator" {
                return Err(Failure::local(
                    "the bundled runner is simulator-only; a physical iPhone must build and sign once",
                    "run `serve` from a source checkout, or set AGENT_MOBILE_DRIVER_DIR to the driver project",
                ));
            }
            cmd.current_dir(dir)
                .arg("test-without-building")
                .arg("-xctestrun")
                .arg(xctestrun.file_name().unwrap_or(xctestrun.as_os_str()))
                .args([
                    "-destination",
                    &sim_destination(device),
                    TEST_ONLY,
                    "-parallel-testing-enabled",
                    "NO",
                ]);
        }
    }
    cmd.env("TEST_RUNNER_AGENT_MOBILE_PORT", port.to_string())
        .env("TEST_RUNNER_AGENT_MOBILE_TOKEN", token);
    if device.kind != "simulator" {
        cmd.env("TEST_RUNNER_AGENT_MOBILE_BIND", "0.0.0.0");
    }
    Ok(cmd)
}

/// The `host:port` a running driver listens on, without a URL scheme —
/// the shape TCP probes want.
#[must_use]
pub fn driver_addr(device: &Device, port: u16) -> String {
    if device.kind == "simulator" {
        format!("127.0.0.1:{port}")
    } else {
        format!("{}:{port}", super::bonjour_host(&device.name))
    }
}

/// The URL a running driver answers on: loopback for a simulator, the
/// device's Bonjour host for a physical iPhone.
#[must_use]
pub fn driver_url(device: &Device, port: u16) -> String {
    format!("http://{}", driver_addr(device, port))
}
