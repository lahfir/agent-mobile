use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;

use crate::adb::{Adb, CommandOutput, CommandRunner};
use crate::testkit::output;

use super::{await_avd_ready, find_avd_row};

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
