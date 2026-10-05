//! Lifecycle package validation, launch ordering, terminate refusal set.

use std::path::PathBuf;
use std::sync::Arc;

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
