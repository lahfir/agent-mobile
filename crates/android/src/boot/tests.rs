use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;

use crate::adb::{Adb, CommandOutput, CommandRunner};
use crate::device::BootedAvd;
use crate::testkit::{FakeRunner, output};

use super::{await_avd_ready, boot_avd_until, find_avd_row, spawn_detached};

/// Devices reply, then a per-call `emu avd name` outcome.
struct Probe {
    devices: &'static str,
    name: Result<&'static str, &'static str>,
}

impl CommandRunner for Probe {
    fn run(
        &self,
        _program: &std::path::Path,
        args: &[&str],
        _timeout: Duration,
    ) -> Result<CommandOutput, Failure> {
        if args.contains(&"emu") {
            return self
                .name
                .map(|r| output(true, r, ""))
                .map_err(|m| Failure::local(m, "check adb"));
        }
        Ok(output(true, self.devices, ""))
    }
}

fn adb_with(p: Probe) -> Adb {
    Adb::with_runner(PathBuf::from("adb"), Arc::new(p))
}

struct Seq(Mutex<VecDeque<CommandOutput>>);

impl CommandRunner for Seq {
    fn run(
        &self,
        _p: &std::path::Path,
        _a: &[&str],
        _t: Duration,
    ) -> Result<CommandOutput, Failure> {
        self.0
            .lock()
            .map_err(|_| Failure::local("lock", "retry"))?
            .pop_front()
            .ok_or_else(|| Failure::local("script exhausted", "add a reply"))
    }
}

#[test]
fn strict_unproven_identity_is_hard_error() {
    let adb = adb_with(Probe {
        devices: "emulator-5554\tdevice\n",
        name: Ok("OK\n"),
    });
    let err = find_avd_row(&adb, "real-avd", true)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("cannot prove AVD identity"), "{err}");
}

#[test]
fn relaxed_unproven_identity_is_pending_not_fatal() -> Result<(), Failure> {
    let adb = adb_with(Probe {
        devices: "emulator-5554\tdevice\n",
        name: Ok("OK\n"),
    });
    assert!(find_avd_row(&adb, "real-avd", false)?.is_none());
    Ok(())
}

#[test]
fn offline_then_device_boots_successfully() -> Result<(), Failure> {
    let replies = Mutex::new(VecDeque::from(vec![
        output(true, "emulator-5554\toffline\n", ""),
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\n", ""),
        output(true, "1\n", ""),
    ]));
    let adb = Adb::with_runner(PathBuf::from("adb"), Arc::new(Seq(replies)));
    let serial = await_avd_ready(
        &adb,
        "real-avd",
        Instant::now() + Duration::from_secs(10),
        None,
        &AtomicBool::new(false),
    )?;
    assert_eq!(serial, "emulator-5554");
    Ok(())
}

fn boot(
    adb: &Adb,
    emulator: &Path,
    name: &str,
    log: &Path,
    budget: Duration,
) -> Result<BootedAvd, Failure> {
    boot_avd_until(adb, emulator, name, log, budget, &AtomicBool::new(false))
}

#[test]
fn boot_rejects_unsafe_and_unknown_names() {
    let runner = FakeRunner::scripted(vec![output(true, "real-avd\n", "")]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    for bad in ["bad;name", "../up", "with space", "-flag"] {
        assert!(
            boot(
                &adb,
                &PathBuf::from("emulator"),
                bad,
                PathBuf::from("/tmp/x").as_path(),
                std::time::Duration::from_secs(1)
            )
            .is_err()
        );
    }
    assert!(
        boot(
            &adb,
            &PathBuf::from("emulator"),
            "not-listed",
            PathBuf::from("/tmp/x").as_path(),
            std::time::Duration::from_secs(1)
        )
        .is_err()
    );
    assert_eq!(runner.calls().len(), 1);
}

#[test]
fn boot_reuses_running_correlated_avd() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\nOK\n", ""),
        output(true, "1\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let booted = boot(
        &adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/am-test-boot.log").as_path(),
        std::time::Duration::from_secs(1),
    )?;
    assert_eq!(booted.serial, "emulator-5554");
    assert!(
        runner
            .calls()
            .iter()
            .all(|c| c.first().is_some_and(|a| a != "-avd"))
    );
    Ok(())
}

#[test]
fn spawn_detached_records_pid_and_log() -> Result<(), Failure> {
    let dir = std::env::temp_dir().join(format!("am-spawn-{}", std::process::id()));
    let log = dir.join("emu.log");
    let pid = spawn_detached(Path::new("/usr/bin/true"), &[], &log)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log)
            .map_err(Failure::from)?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "log must be 0600, got {mode:o}");
    }
    assert!(pid > 0);
    assert!(log.exists());
    Ok(())
}

#[test]
fn boot_waits_for_boot_completed_on_running_avd() -> Result<(), Failure> {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\nOK\n", ""),
        output(true, "0", ""),
        output(true, "emulator-5554\tdevice\n", ""),
        output(true, "real-avd\nOK\n", ""),
        output(true, "1", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let booted = boot(
        &adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/x").as_path(),
        std::time::Duration::from_secs(5),
    )?;
    assert_eq!(booted.serial, "emulator-5554");
    assert_eq!(booted.pid, 0);
    assert_eq!(runner.calls().len(), 7);
    Ok(())
}

#[test]
fn boot_fails_for_unauthorized_running_row() {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(true, "emulator-5554\tunauthorized\n", ""),
        output(true, "real-avd\nOK\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let err = boot(
        &adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/x").as_path(),
        std::time::Duration::from_secs(5),
    )
    .err()
    .map(|e| format!("{} {}", e.message(), e.render()))
    .unwrap_or_default();
    assert!(err.contains("unauthorized"), "missing remedy: {err}");
}

#[test]
fn boot_fails_when_devices_probe_fails() {
    let runner = FakeRunner::scripted(vec![
        output(true, "real-avd\n", ""),
        output(false, "", "daemon refused"),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    assert!(
        boot(
            &adb,
            &PathBuf::from("emulator"),
            "real-avd",
            PathBuf::from("/tmp/x").as_path(),
            std::time::Duration::from_secs(5),
        )
        .is_err()
    );
    assert_eq!(runner.calls().len(), 2);
}

#[test]
fn boot_avd_until_cancelled_errors_before_next_probe() -> Result<(), Failure> {
    let cancelled = AtomicBool::new(true);
    let runner = FakeRunner::scripted(vec![
        output(true, "agent-mobile-api37\n", ""),
        output(
            true,
            "emulator-5554\tdevice product:x model:y device:z transport_id:1\n",
            "",
        ),
        output(true, "agent-mobile-api37\nOK\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner);
    let log = std::env::temp_dir().join("am-cancel-test.log");
    let err = crate::boot::boot_avd_until(
        &adb,
        PathBuf::from("emulator").as_path(),
        "agent-mobile-api37",
        &log,
        Duration::from_secs(30),
        &cancelled,
    )
    .err()
    .ok_or_else(|| Failure::local("cancel did not error", "fail"))?;
    assert!(format!("{err:?}").contains("cancelled"), "{err:?}");
    Ok(())
}

#[test]
fn precancelled_boot_never_spawns_avd() {
    let cancelled = AtomicBool::new(true);
    let runner = FakeRunner::scripted(vec![
        output(true, "agent-mobile-api37\n", ""),
        output(true, "List of devices attached\n\n", ""),
    ]);
    let adb = Adb::with_runner(PathBuf::from("adb"), runner.clone());
    let log = std::env::temp_dir().join("am-precancel-test.log");
    let err = crate::boot::boot_avd_until(
        &adb,
        PathBuf::from("emulator").as_path(),
        "agent-mobile-api37",
        &log,
        Duration::from_secs(30),
        &cancelled,
    );
    assert!(err.is_err());
    assert!(
        !runner.calls().iter().any(|c| c.iter().any(|a| a == "-avd")),
        "no emulator -avd command may issue when pre-cancelled"
    );
}

struct BootProbe {
    emu: Result<&'static str, &'static str>,
}

impl crate::adb::CommandRunner for BootProbe {
    fn run(
        &self,
        _program: &std::path::Path,
        args: &[&str],
        _timeout: std::time::Duration,
    ) -> Result<crate::adb::CommandOutput, Failure> {
        if args.contains(&"emu") {
            return self
                .emu
                .map(|reply| output(true, reply, ""))
                .map_err(|m| Failure::local(m, "check adb"));
        }
        if args.contains(&"-list-avds") {
            return Ok(output(true, "real-avd\n", ""));
        }
        Ok(output(true, "emulator-5554\tdevice\n", ""))
    }
}

fn boot_err(adb: &Adb) -> String {
    boot(
        adb,
        &PathBuf::from("emulator"),
        "real-avd",
        PathBuf::from("/tmp/am-test-boot.log").as_path(),
        std::time::Duration::from_secs(1),
    )
    .err()
    .map(|e| e.render())
    .unwrap_or_default()
}

#[test]
fn boot_propagates_live_row_identity_probe_failure() {
    let adb = Adb::with_runner(
        PathBuf::from("adb"),
        std::sync::Arc::new(BootProbe {
            emu: Err("emu probe refused"),
        }),
    );
    assert!(boot_err(&adb).contains("emu probe refused"));
}

#[test]
fn boot_fails_closed_on_unprovable_identity() {
    let adb = Adb::with_runner(
        PathBuf::from("adb"),
        std::sync::Arc::new(BootProbe { emu: Ok("OK\n") }),
    );
    assert!(boot_err(&adb).contains("cannot prove AVD identity"));
}

#[test]
fn boot_foreign_name_row_is_non_match_not_fatal() {
    let adb = Adb::with_runner(
        PathBuf::from("adb"),
        std::sync::Arc::new(BootProbe {
            emu: Ok("other-avd\nOK\n"),
        }),
    );
    let err = boot_err(&adb);
    assert!(!err.contains("cannot prove") && !err.is_empty(), "{err}");
}
