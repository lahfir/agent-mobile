//! Headless AVD boot: detached spawn with a background reaper, then a
//! bounded poll for a correlated row to reach `device` +
//! `sys.boot_completed=1`.

use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::process::open_private_log;

use crate::adb::Adb;
use crate::device::{
    AndroidDeviceKind, BootedAvd, DeviceRow, avd_name, kind_of, list_avds, parse_devices,
};

/// Build the emulator `Command`: null stdin, log on both pipes, and on
/// Unix its own process group so a terminal SIGINT can never reach it.
fn detached_command(program: &Path, args: &[&str], stdout: File, stderr: File) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

/// Spawn `program` detached with output appended to a private `0600` log;
/// returns the pid. The child handle is dropped, never killed.
///
/// # Errors
/// [`Failure::Local`] on log-open or spawn failures.
pub(crate) fn spawn_detached(program: &Path, args: &[&str], log: &Path) -> Result<u32, Failure> {
    let file = open_private_log(log)?;
    let err_file = File::try_clone(&file).map_err(Failure::from)?;
    let mut child = detached_command(program, args, file, err_file)
        .spawn()
        .map_err(|e| {
            Failure::local(
                format!("cannot spawn {}: {e}", program.display()),
                "check the emulator binary and retry",
            )
        })?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// Is `name` a safe AVD identifier? First char alphanumeric so `-flag`
/// shapes can never become options.
fn valid_avd_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Find the emulator row reporting `name` as its AVD, keeping its state.
/// A failed `devices` probe is a hard error, never "not running".
fn find_avd_row(adb: &Adb, name: &str) -> Result<Option<DeviceRow>, Failure> {
    let out = adb.unscoped(&["devices"])?;
    if !out.success {
        return Err(Failure::local(
            format!("adb devices failed: {}", out.stderr.trim()),
            "restart the adb server and retry",
        ));
    }
    for row in parse_devices(&out.stdout) {
        if kind_of(&row.serial) != AndroidDeviceKind::Emulator {
            continue;
        }
        if let Ok(Some(found)) = avd_name(adb, &row.serial)
            && found == name
        {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// Poll until a correlated row is `device` and `sys.boot_completed` is `1`.
/// Offline/unauthorized rows surface an executable remedy; the deadline is
/// `budget` from `started`.
fn await_avd_ready(
    adb: &Adb,
    name: &str,
    deadline: Instant,
    first: Option<DeviceRow>,
) -> Result<String, Failure> {
    let mut pending = first;
    loop {
        let found = match pending.take() {
            Some(row) => Some(row),
            None => find_avd_row(adb, name)?,
        };
        if let Some(row) = found {
            match row.state.as_str() {
                "device" if boot_completed(adb, &row.serial) => return Ok(row.serial),
                "offline" | "unauthorized" => {
                    return Err(Failure::local(
                        format!("AVD {name:?} is {} on {}", row.state, row.serial),
                        "accept the USB debugging prompt or run `adb reconnect` and retry",
                    ));
                }
                _ => {}
            }
        }
        if Instant::now() > deadline {
            return Err(Failure::local(
                format!("AVD {name:?} did not finish booting in time"),
                "check the emulator log and `adb devices` state",
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Whether `sys.boot_completed` reports `1` on `serial`.
fn boot_completed(adb: &Adb, serial: &str) -> bool {
    adb.scoped(serial, &["shell", "getprop", "sys.boot_completed"])
        .map(|o| o.stdout.trim() == "1")
        .unwrap_or(false)
}

/// Boot `name` headless, or reuse it when an emulator already reports it.
/// Bounded by `budget`; offline/unauthorized states surface remedies.
///
/// # Errors
/// [`Failure::Local`] on unsafe/unknown names, spawn failure, or a boot
/// that never reaches `sys.boot_completed=1` inside `budget`.
pub(crate) fn boot_avd(
    adb: &Adb,
    emulator: &Path,
    name: &str,
    log: &Path,
    budget: Duration,
) -> Result<BootedAvd, Failure> {
    if !valid_avd_name(name) {
        return Err(Failure::usage(format!("invalid AVD name {name:?}")));
    }
    let configured = list_avds(adb, emulator)?;
    if !configured.iter().any(|a| a == name) {
        return Err(Failure::local(
            format!("unknown AVD {name:?}"),
            "list available AVDs with `emulator -list-avds` and pick one",
        ));
    }
    let deadline = Instant::now() + budget;
    let first = find_avd_row(adb, name)?;
    let mut pid = 0;
    if first.is_none() {
        pid = spawn_detached(
            emulator,
            &[
                "-avd",
                name,
                "-no-window",
                "-no-audio",
                "-no-boot-anim",
                "-gpu",
                "swiftshader_indirect",
            ],
            log,
        )?;
    }
    let serial = await_avd_ready(adb, name, deadline, first)?;
    Ok(BootedAvd {
        serial,
        pid,
        log: log.to_path_buf(),
    })
}
