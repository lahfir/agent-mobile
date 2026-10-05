//! APK install, private provisioning, service merge, owned forwards.

use std::path::PathBuf;
use std::sync::Arc;

use agent_mobile_core::error::Failure;

use super::{
    DEVICE_PORT, PACKAGE, SERVICE_COMPONENT, enable_service, enable_service_bounded, ensure_apk,
    install, merge_enabled_services, provision,
};
use crate::adb::{Adb, CommandOutput};
use crate::forward::{create_forward, remove_forward};
use crate::testkit::{FakeRunner, output};

const TOKEN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
const PROVISION: &str = "result=Bundle[{token=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq}]";

fn adb_with(replies: Vec<CommandOutput>) -> (Adb, Arc<FakeRunner>) {
    let runner = FakeRunner::scripted(replies);
    (
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        runner,
    )
}

#[test]
fn provision_extracts_token_and_redacts() -> Result<(), Failure> {
    let (adb, _) = adb_with(vec![output(true, PROVISION, "")]);
    let token = provision(&adb, "s1")?;
    assert_eq!(token.as_str(), TOKEN);
    assert_eq!(format!("{token:?}"), "<redacted>");
    assert_eq!(format!("{token}"), "<redacted>");
    Ok(())
}

#[test]
fn provision_failure_never_echoes_output() {
    let (adb, _) = adb_with(vec![output(
        true,
        "result=Bundle[{notoken=oops-sentinel}]",
        "",
    )]);
    let err = provision(&adb, "s1").err().map(|e| format!("{e:?}"));
    let text = err.unwrap_or_default();
    assert!(!text.contains("oops-sentinel"));
}

#[test]
fn merge_preserves_order_dedupes_and_appends() {
    assert_eq!(
        merge_enabled_services("", SERVICE_COMPONENT),
        SERVICE_COMPONENT
    );
    assert_eq!(
        merge_enabled_services("null", SERVICE_COMPONENT),
        SERVICE_COMPONENT
    );
    let merged = merge_enabled_services("a.b/.C:d.e/.F", SERVICE_COMPONENT);
    assert_eq!(merged, format!("a.b/.C:d.e/.F:{SERVICE_COMPONENT}"));
    let again = merge_enabled_services(&merged, SERVICE_COMPONENT);
    assert_eq!(again, merged);
    let dup = merge_enabled_services("a.b/.C:a.b/.C", SERVICE_COMPONENT);
    assert_eq!(dup, format!("a.b/.C:{SERVICE_COMPONENT}"));
}

#[test]
fn enable_skips_writes_when_already_set() -> Result<(), Failure> {
    let existing = format!("a.b/.C:{SERVICE_COMPONENT}");
    let replies = vec![
        output(true, &existing, ""),
        output(true, "1", ""),
        output(true, &existing, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
    ];
    let (adb, runner) = adb_with(replies);
    enable_service(&adb, "s1")?;
    let calls = runner.calls();
    assert!(!calls.iter().any(|c| c.contains(&"put".to_owned())));
    Ok(())
}

#[test]
fn enable_writes_only_the_diff() -> Result<(), Failure> {
    let existing = "a.b/.C";
    let merged = format!("{existing}:{SERVICE_COMPONENT}");
    let replies = vec![
        output(true, existing, ""),
        output(true, "", ""),
        output(true, "1", ""),
        output(true, &merged, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
    ];
    let (adb, runner) = adb_with(replies);
    enable_service(&adb, "s1")?;
    let calls = runner.calls();
    let puts: Vec<_> = calls
        .iter()
        .filter(|c| c.contains(&"put".to_owned()))
        .collect();
    assert_eq!(puts.len(), 1);
    assert!(puts[0].iter().any(|a| a == &merged));
    assert!(calls.iter().all(|c| c[0] == "-s" && c[1] == "s1"));
    Ok(())
}

#[test]
fn install_incompatible_gives_explicit_remedy() {
    let (adb, runner) = adb_with(vec![output(
        false,
        "adb: failed",
        "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE]",
    )]);
    let err = install(
        &adb,
        "emulator-5554",
        PathBuf::from("/x/app-debug.apk").as_path(),
    );
    let text = format!("{:?}", err.err());
    assert!(text.contains(&format!("adb -s emulator-5554 uninstall {PACKAGE}")));
    let calls = runner.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][2], "install");
}

#[test]
fn forward_roundtrip_owns_exact_row() -> Result<(), Failure> {
    let replies = vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "58285", ""),
        output(
            true,
            &format!("emulator-5554 tcp:58285 tcp:{DEVICE_PORT}\nother tcp:1 tcp:2"),
            "",
        ),
        output(true, "", ""),
    ];
    let (adb, runner) = adb_with(replies);
    let port = create_forward(&adb, "emulator-5554")?;
    assert_eq!(port, 58285);
    remove_forward(&adb, "emulator-5554", port)?;
    let calls = runner.calls();
    let last = &calls[3];
    assert_eq!(
        last,
        &["-s", "emulator-5554", "forward", "--remove", "tcp:58285"]
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
        output(true, "59999", ""),
        output(
            true,
            "emulator-5554 tcp:7000 tcp:8770\nother tcp:1 tcp:2",
            "",
        ),
        output(true, "", ""),
    ]);
    assert!(create_forward(&adb, "emulator-5554").is_err());
    let calls = runner.calls();
    assert!(calls.iter().any(|c| {
        c.iter().map(String::as_str).collect::<Vec<_>>()
            == ["-s", "emulator-5554", "forward", "--remove", "tcp:59999"]
    }));
    assert!(
        !calls
            .iter()
            .any(|c| c.iter().any(|a| a == "tcp:7000" || a == "tcp:1"))
    );
}

#[test]
fn apk_env_override_wins() -> Result<(), Failure> {
    let dir = std::env::temp_dir().join(format!("am-apk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(Failure::from)?;
    let apk = dir.join("driver.apk");
    std::fs::write(&apk, b"apk").map_err(Failure::from)?;
    let runner: std::sync::Arc<dyn crate::adb::CommandRunner> = FakeRunner::scripted(vec![]);
    let got = ensure_apk(
        Some(apk.as_path()),
        PathBuf::from("/missing").as_path(),
        &runner,
    )?;
    assert_eq!(got, apk);
    Ok(())
}

#[test]
fn token_parse_requires_exact_bundle_field() {
    let (adb, _r) = adb_with(vec![
        output(true, &format!("Bundle[{{token={TOKEN}}}]"), ""),
        output(true, &format!("Bundle[{{not_token={TOKEN}}}]"), ""),
        output(true, &format!("Bundle[{{token={}}}]", &TOKEN[..42]), ""),
        output(true, &format!("Bundle[{{token={TOKEN}x}}]"), ""),
        output(true, &format!("Bundle[{{token={}*}}]", &TOKEN[..42]), ""),
        output(
            true,
            &format!("Bundle[{{a=1,token={TOKEN},token={TOKEN}}}]"),
            "",
        ),
    ]);
    assert!(provision(&adb, "s1").is_ok(), "valid field rejected");
    for _ in 0..5 {
        let err = provision(&adb, "s1").map(|_| ());
        assert!(err.is_err(), "malformed field accepted");
        let text = err.err().map(|e| e.render()).unwrap_or_default();
        assert!(!text.contains(TOKEN), "error leaked token: {text}");
    }
}

#[test]
fn verify_requires_exact_component_entry() {
    let merged = format!("a.b/.C:{SERVICE_COMPONENT}");
    let (adb, _r) = adb_with(vec![
        output(true, "a.b/.C", ""),
        output(true, "", ""),
        output(true, "1", ""),
        output(
            true,
            "evil.x/com.lahfir.agentmobile.driver.AgentMobileAccessibilityService.evil",
            "",
        ),
        output(true, &merged, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
    ]);
    assert!(
        enable_service(&adb, "s1").is_err(),
        "substring entry accepted"
    );
}

#[test]
fn verify_accepts_short_form_component() -> Result<(), Failure> {
    let (adb, runner) = adb_with(vec![
        output(
            true,
            "a.b/.C:com.lahfir.agentmobile.driver/.AgentMobileAccessibilityService",
            "",
        ),
        output(true, "1", ""),
        output(
            true,
            "a.b/.C:com.lahfir.agentmobile.driver/.AgentMobileAccessibilityService",
            "",
        ),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
    ]);
    enable_service(&adb, "s1")?;
    assert_eq!(runner.calls().len(), 4);
    Ok(())
}

#[test]
fn dumpsys_needs_bound_service_label() {
    let merged = format!("a.b/.C:{SERVICE_COMPONENT}");
    let (adb, _r) = adb_with(vec![
        output(true, &merged, ""),
        output(true, "1", ""),
        output(true, &merged, ""),
        output(
            true,
            "Bound services:{} Enabled services:{[AgentMobileAccessibilityService]}",
            "",
        ),
    ]);
    assert!(
        enable_service_bounded(&adb, "s1", std::time::Duration::ZERO).is_err(),
        "enabled-services wording counted as bound"
    );
}

#[test]
fn put_refusal_names_restricted_settings() {
    let (adb, _r) = adb_with(vec![
        output(true, "a.b/.C", ""),
        output(false, "", "Permission denial"),
    ]);
    let err = enable_service(&adb, "s1")
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("Allow restricted settings"), "{err}");
    assert!(err.contains("Accessibility"), "{err}");
    assert!(err.contains("Agent Mobile Driver"), "{err}");
}

#[test]
fn forward_cleanup_on_list_failure() {
    let (adb, runner) = adb_with(vec![
        output(true, "", ""),
        output(true, "59999", ""),
        output(false, "", "list broke"),
        output(true, "", ""),
    ]);
    assert!(create_forward(&adb, "emulator-5554").is_err());
    let calls = runner.calls();
    assert!(
        calls.iter().any(|c| c
            == &vec![
                "-s".to_owned(),
                "emulator-5554".to_owned(),
                "forward".to_owned(),
                "--remove".to_owned(),
                "tcp:59999".to_owned(),
            ]),
        "{calls:?}"
    );
}

#[test]
fn forward_malformed_port_diffs_rows() {
    let (adb, runner) = adb_with(vec![
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "not-a-port", ""),
        output(
            true,
            "other tcp:1 tcp:2\nemulator-5554 tcp:9999 tcp:8770",
            "",
        ),
        output(true, "", ""),
    ]);
    assert!(create_forward(&adb, "emulator-5554").is_err());
    let calls = runner.calls();
    assert!(calls.iter().any(|c| c.iter().any(|a| a == "tcp:9999")));
    assert!(!calls.iter().any(|c| c.iter().any(|a| a == "tcp:1")));
}
