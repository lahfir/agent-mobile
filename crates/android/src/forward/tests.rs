use std::path::PathBuf;
use std::sync::Arc;

use agent_mobile_core::error::Failure;

use super::{create_forward, remove_owned_forward};
use crate::adb::{Adb, CommandOutput, CommandRunner};
use crate::driver::LEGACY_DEVICE_PORT;
use crate::testkit::{FakeRunner, output};

const LOCAL: u16 = 9999;

fn adb_with(replies: Vec<CommandOutput>) -> (Adb, Arc<FakeRunner>) {
    let runner = FakeRunner::scripted(replies);
    (
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        runner,
    )
}

#[test]
fn forward_roundtrip_owns_exact_row() -> Result<(), Failure> {
    let replies = vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "", ""),
        output(
            true,
            &format!("emulator-5554 tcp:9999 tcp:{LEGACY_DEVICE_PORT}\nother tcp:1 tcp:2"),
            "",
        ),
        output(
            true,
            &format!("emulator-5554 tcp:9999 tcp:{LEGACY_DEVICE_PORT}\nother tcp:1 tcp:2"),
            "",
        ),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
    ];
    let (adb, runner) = adb_with(replies);
    let port = create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT)?;
    assert_eq!(port, LOCAL);
    remove_owned_forward(&adb, "emulator-5554", port, LEGACY_DEVICE_PORT)?;
    let calls = runner.calls();
    let last = calls
        .iter()
        .rfind(|c| c.contains(&"--remove".to_owned()))
        .ok_or_else(|| Failure::local("remove never issued", "fail"))?;
    assert_eq!(
        last,
        &["-s", "emulator-5554", "forward", "--remove", "tcp:9999"]
    );
    assert!(!last.iter().any(|a| a == &"tcp:1".to_owned()));
    Ok(())
}
#[test]
fn forward_fails_when_row_absent() {
    let (adb, runner) = adb_with(vec![
        output(
            true,
            "emulator-5554 tcp:7000 tcp:8770\nother tcp:1 tcp:2",
            "",
        ),
        output(true, "", ""),
        output(
            true,
            "emulator-5554 tcp:7000 tcp:8770\nother tcp:1 tcp:2",
            "",
        ),
        output(
            true,
            "emulator-5554 tcp:7000 tcp:8770\nother tcp:1 tcp:2",
            "",
        ),
    ]);
    let err = create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT);
    assert!(err.is_err());
    let calls = runner.calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.iter().any(|a| a == &"--remove".to_owned())),
        "blind remove on a row absent from the list must never run: {calls:?}"
    );
}
#[test]
fn forward_cleanup_on_list_failure() {
    let (adb, _r) = adb_with(vec![
        output(true, "", ""),
        output(true, "", ""),
        output(false, "", "list broke"),
        output(false, "", "cleanup list broke"),
    ]);
    let err = create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("list broke"), "cause lost: {err}");
    assert!(
        err.contains("cleanup list broke"),
        "cleanup failure hidden: {err}"
    );
    for needle in [
        "emulator-5554",
        "tcp:9999",
        "tcp:8770",
        "confirm remote tcp:8770, then `adb -s emulator-5554 forward --remove tcp:9999`",
    ] {
        assert!(err.contains(needle), "missing {needle}: {err}");
    }
}

#[test]
fn cleanup_errors_on_malformed_owned_local() {
    let (adb, _r) = adb_with(vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:abc tcp:8770",
            "",
        ),
    ]);
    let err = create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(
        err.contains("cleanup"),
        "malformed owned row must fail: {err}"
    );
    assert!(err.contains("tcp:abc"), "row identity lost: {err}");
}
#[test]
fn forward_malformed_port_diffs_rows() {
    let (adb, runner) = adb_with(vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:8770",
            "",
        ),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:8770",
            "",
        ),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
    ]);
    assert!(create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT).is_err());
    let calls = runner.calls();
    assert!(calls.iter().any(|c| {
        c.iter().map(String::as_str).collect::<Vec<_>>()
            == ["-s", "emulator-5554", "forward", "--remove", "tcp:9999"]
    }));
    assert!(!calls.iter().any(|c| c.iter().any(|a| a == "tcp:1")));
}
#[test]
fn cleanup_never_removes_a_foreign_remote() {
    let (adb, runner) = adb_with(vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "", ""),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:4444",
            "",
        ),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:4444",
            "",
        ),
    ]);
    assert!(create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT).is_err());
    let calls = runner.calls();
    assert!(
        !calls.iter().any(|c| {
            c.iter().any(|a| a == &"--remove".to_owned())
                && c.iter().any(|a| a == &"tcp:9999".to_owned())
        }),
        "foreign-remote row must never be removed: {calls:?}"
    );
}

#[test]
fn remove_error_but_row_gone_is_success() {
    let (adb, _r) = adb_with(vec![
        output(true, "emulator-5554 tcp:5000 tcp:8770", ""),
        output(false, "", "remove exploded"),
        output(true, "other tcp:1 tcp:2", ""),
    ]);
    assert!(remove_owned_forward(&adb, "emulator-5554", 5000, LEGACY_DEVICE_PORT).is_ok());
}

#[test]
fn cancelled_alloc_still_runs_uncancelled_cleanup() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct FlipRunner {
        inner: Arc<FakeRunner>,
        flag: Arc<AtomicBool>,
    }
    impl CommandRunner for FlipRunner {
        fn run(
            &self,
            program: &std::path::Path,
            args: &[&str],
            timeout: std::time::Duration,
        ) -> Result<CommandOutput, Failure> {
            if args.iter().any(|a| a.starts_with("tcp:0")) {
                self.flag.store(true, Ordering::Relaxed);
            }
            self.inner.run(program, args, timeout)
        }
    }
    let flag = Arc::new(AtomicBool::new(false));
    let inner = FakeRunner::scripted(vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(false, "", "interrupted"),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:8770",
            "",
        ),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:8770",
            "",
        ),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
    ]);
    let calls = inner.clone();
    let adb = Adb::with_runner(
        PathBuf::from("adb"),
        Arc::new(FlipRunner {
            inner,
            flag: flag.clone(),
        }),
    )
    .with_cancellation(flag);
    let err = create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(
        err.contains("forward") || err.contains("interrupted"),
        "{err}"
    );
    assert!(calls.calls().iter().any(|call| {
        call.iter().map(String::as_str).collect::<Vec<_>>()
            == ["-s", "emulator-5554", "forward", "--remove", "tcp:9999"]
    }));
}

#[test]
fn forward_cleanup_finds_owned_row_past_big_list() {
    let filler = (0..200).fold(String::new(), |mut acc, i| {
        use std::fmt::Write as _;
        let _ = writeln!(acc, "other-{i} tcp:{} tcp:9{i:03}", 20000 + i);
        acc
    });
    let (adb, runner) = adb_with(vec![
        output(true, &filler, ""),
        output(true, "", ""),
        output(true, &filler, ""),
        output(
            true,
            &format!("{filler}emulator-5554 tcp:9999 tcp:8770"),
            "",
        ),
        output(
            true,
            &format!("{filler}emulator-5554 tcp:9999 tcp:8770"),
            "",
        ),
        output(true, "", ""),
        output(true, &filler, ""),
    ]);
    assert!(create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT).is_err());
    let calls = runner.calls();
    assert!(calls.iter().any(|c| {
        c.iter().map(String::as_str).collect::<Vec<_>>()
            == ["-s", "emulator-5554", "forward", "--remove", "tcp:9999"]
    }));
}

#[test]
fn malformed_list_row_fails_closed_no_remove() {
    let (adb, runner) = adb_with(vec![
        output(true, "emulator-5554 tcp:1\nother tcp:2 tcp:3", ""),
        output(true, "emulator-5554 tcp:1", ""),
    ]);
    let err = create_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("malformed"), "{err}");
    assert!(
        !runner
            .calls()
            .iter()
            .any(|c| c.iter().any(|a| a == "--remove")),
        "no remove may be attempted past a malformed list"
    );
    let err = remove_owned_forward(&adb, "emulator-5554", LOCAL, LEGACY_DEVICE_PORT)
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(
        err.contains("malformed"),
        "malformed list must surface, not report success: {err}"
    );
}
