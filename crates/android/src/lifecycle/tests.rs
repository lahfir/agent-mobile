//! Lifecycle package validation, launch ordering, terminate refusal set.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::{AdbLifecycle, LifecycleControl, LifecycleError, valid_package};
use crate::adb::Adb;
use crate::testkit::{FakeRunner, output};

fn ctl_with(replies: Vec<crate::adb::CommandOutput>) -> (AdbLifecycle, Arc<FakeRunner>) {
    let runner = FakeRunner::scripted(replies);
    (
        AdbLifecycle::new(Adb::with_runner(PathBuf::from("adb"), runner.clone()), "s1"),
        runner,
    )
}

#[test]
fn package_names_must_be_java_identifiers() {
    for good in ["com.a.b", "a", "com.pkg_9.X2"] {
        assert!(valid_package(good), "{good} should pass");
    }
    for bad in [
        "", "a b", "a;b", "a/b", "-flag", "a..b", ".a", "a.", "9bad", "com.*",
    ] {
        assert!(!valid_package(bad), "{bad} should fail");
    }
}

#[test]
fn launch_resolves_stops_and_starts_in_order() -> Result<(), LifecycleError> {
    let (ctl, runner) = ctl_with(vec![
        output(true, "priority=0\ncom.pkg.app/.MainActivity\n", ""),
        output(true, "", ""),
        output(true, "Status: ok\nActivity: com.pkg.app/.MainActivity", ""),
    ]);
    ctl.launch("com.pkg.app")?;
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    let resolve = &calls[0];
    assert!(resolve[1..].windows(1).any(|w| w[0] == "s1"));
    assert!(resolve.iter().any(|a| a.contains("resolve-activity")));
    assert_eq!(
        calls[1][3], "'am' 'force-stop' 'com.pkg.app'",
        "remote words arrive as one quoted shell string: {:?}",
        calls[1]
    );
    assert!(
        calls[2][3].contains("'com.pkg.app/.MainActivity'"),
        "{:?}",
        calls[2]
    );
    Ok(())
}

#[test]
fn launch_rejects_injection_before_any_command() {
    let (ctl, runner) = ctl_with(vec![]);
    assert!(matches!(
        ctl.launch("a;rm -rf /"),
        Err(LifecycleError::BadRequest(_))
    ));
    assert!(runner.calls().is_empty());
}

#[test]
fn launch_maps_error_output_to_driver_error() {
    let (ctl, _) = ctl_with(vec![
        output(true, "com.pkg.app/.MainActivity", ""),
        output(true, "", ""),
        output(true, "Error: Activity not started", ""),
    ]);
    assert!(matches!(
        ctl.launch("com.pkg.app"),
        Err(LifecycleError::Driver(_))
    ));
}

#[test]
fn terminate_refuses_system_and_launcher() {
    let (ctl, _) = ctl_with(vec![output(true, "com.launcher/.Home\n", "")]);
    for pkg in [
        "",
        "android",
        crate::driver::PACKAGE,
        "com.android.systemui",
        "com.launcher",
    ] {
        assert!(
            matches!(ctl.terminate(pkg), Err(LifecycleError::BadRequest(_))),
            "{pkg}"
        );
    }
}

#[test]
fn terminate_force_stops_only_observed_package() -> Result<(), LifecycleError> {
    let (ctl, runner) = ctl_with(vec![
        output(true, "com.launcher/.Home\n", ""),
        output(true, "", ""),
    ]);
    ctl.terminate("com.target.app")?;
    let calls = runner.calls();
    let last = &calls[1];
    let remote = &last[3];
    assert!(remote.contains("'force-stop'"), "{remote}");
    assert!(remote.contains("'com.target.app'"), "{remote}");
    assert!(!remote.contains("com.launcher"), "{remote}");
    Ok(())
}

#[test]
fn error_inside_component_does_not_fail() -> Result<(), LifecycleError> {
    let (ctl, runner) = ctl_with(vec![
        output(true, "com.pkg.app/.ErrorActivity", ""),
        output(true, "", ""),
        output(true, "Status: ok", ""),
    ]);
    ctl.launch("com.pkg.app")?;
    assert_eq!(runner.calls().len(), 3);
    Ok(())
}

#[test]
fn exception_line_fails() {
    let (ctl, _) = ctl_with(vec![
        output(true, "com.pkg.app/.MainActivity", ""),
        output(true, "", ""),
        output(true, "Exception: occurred while executing", ""),
    ]);
    assert!(matches!(
        ctl.launch("com.pkg.app"),
        Err(LifecycleError::Driver(_))
    ));
}

#[test]
fn launch_rejects_shell_in_class_before_any_mutation() {
    let (ctl, runner) = ctl_with(vec![output(true, "com.pkg.app/.Main;id\n", "")]);
    assert!(matches!(
        ctl.launch("com.pkg.app"),
        Err(LifecycleError::BadRequest(_))
    ));
    assert_eq!(
        runner.calls().len(),
        1,
        "only resolve ran — no force-stop/start: {:?}",
        runner.calls()
    );
}

#[test]
fn launch_rejects_foreign_component_package() {
    let (ctl, runner) = ctl_with(vec![output(true, "com.other/.Main\n", "")]);
    assert!(matches!(
        ctl.launch("com.pkg.app"),
        Err(LifecycleError::BadRequest(_))
    ));
    assert_eq!(runner.calls().len(), 1);
}

#[test]
fn launch_uses_extended_deadline_only_for_start() -> Result<(), LifecycleError> {
    let (ctl, runner) = ctl_with(vec![
        output(true, "priority=0\ncom.pkg.app/.MainActivity\n", ""),
        output(true, "", ""),
        output(true, "Status: ok\nActivity: com.pkg.app/.MainActivity", ""),
    ]);
    ctl.launch("com.pkg.app")?;
    assert_eq!(
        runner.timeouts(),
        vec![
            Duration::from_secs(15),
            Duration::from_secs(15),
            Duration::from_secs(60)
        ],
        "resolve and force-stop keep the default; am start -W gets 60s"
    );
    Ok(())
}

#[test]
fn failed_start_is_driver_error_and_runs_start_once() {
    let (ctl, runner) = ctl_with(vec![
        output(true, "priority=0\ncom.pkg.app/.MainActivity\n", ""),
        output(true, "", ""),
        output(false, "", "Error: Activity not started"),
    ]);
    let result = ctl.launch("com.pkg.app");
    assert!(
        matches!(result.as_ref(), Err(LifecycleError::Driver(_))),
        "{result:?}"
    );
    let starts = runner
        .calls()
        .iter()
        .filter(|c| c.iter().any(|a| a.contains("'start'")))
        .count();
    assert_eq!(1, starts, "no retry of a side-effecting launch");
}

#[test]
fn cancellation_reaches_explicit_timeout_remote_shell() {
    let runner = FakeRunner::scripted(vec![]);
    let flag = Arc::new(AtomicBool::new(true));
    let adb =
        Adb::with_runner(PathBuf::from("adb"), runner.clone()).with_cancellation(flag.clone());
    let result = adb.remote_shell_with(
        "s1",
        &["am", "start", "-W", "-n", "com.pkg/.Main"],
        Duration::from_secs(60),
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.message().contains("interrupted")),
        "{result:?}"
    );
    assert!(
        runner.calls().is_empty(),
        "cancellation short-circuits before spawn: {:?}",
        runner.calls()
    );
}
