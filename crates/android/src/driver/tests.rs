//! APK install, private provisioning, service merge, owned forwards.

use std::path::PathBuf;
use std::sync::Arc;

use agent_mobile_core::error::Failure;

use super::{
    PACKAGE, SERVICE_COMPONENT, enable_service, enable_service_bounded, ensure_apk, install,
    merge_enabled_services, provision,
};
use crate::adb::{Adb, CommandOutput};
use crate::testkit::{FakeRunner, output};

const TOKEN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
const PROVISION: &str =
    "result=Bundle[{token=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq port=9876}]";

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
    let got = provision(&adb, "s1")?;
    assert_eq!(got.token.as_str(), TOKEN);
    assert_eq!(got.device_port, 9876);
    assert_eq!(format!("{:?}", got.token), "<redacted>");
    assert_eq!(format!("{}", got.token), "<redacted>");
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
    assert!(!calls.iter().any(|c| c.iter().any(|a| a.contains("'put'"))));
    assert!(
        !calls
            .iter()
            .any(|c| c.iter().any(|a| a.contains("'getprop'"))),
        "already-enabled path must not probe qemu: {calls:?}"
    );
    Ok(())
}

#[test]
fn enable_writes_only_the_diff() -> Result<(), Failure> {
    let existing = "a.b/.C";
    let merged = format!("{existing}:{SERVICE_COMPONENT}");
    let replies = vec![
        output(true, existing, ""),
        output(true, "1", ""),
        output(true, "1", ""),
        output(true, existing, ""),
        output(true, "", ""),
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
        .filter(|c| c.iter().any(|a| a.contains("'put'")))
        .collect();
    assert_eq!(puts.len(), 1);
    assert!(puts[0].iter().any(|a| a.contains(&format!("'{merged}'"))));
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
fn apk_env_override_wins() -> Result<(), Failure> {
    let dir = std::env::temp_dir().join(format!("am-apk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(Failure::from)?;
    let apk = dir.join("driver.apk");
    std::fs::write(&apk, b"apk").map_err(Failure::from)?;
    let (adb, _r) = adb_with(vec![]);
    let got = ensure_apk(
        Some(apk.as_path()),
        PathBuf::from("/missing").as_path(),
        &adb,
    )?;
    assert_eq!(got, apk);
    Ok(())
}

#[test]
fn token_parse_requires_exact_bundle_field() {
    let (adb, _r) = adb_with(vec![
        output(true, &format!("Bundle[{{token={TOKEN} port=9}}]"), ""),
        output(true, &format!("Bundle[{{not_token={TOKEN} port=9}}]"), ""),
        output(
            true,
            &format!("Bundle[{{token={} port=9}}]", &TOKEN[..42]),
            "",
        ),
        output(true, &format!("Bundle[{{token={TOKEN}x port=9}}]"), ""),
        output(
            true,
            &format!("Bundle[{{token={}* port=9}}]", &TOKEN[..42]),
            "",
        ),
        output(
            true,
            &format!("Bundle[{{a=1,token={TOKEN},token={TOKEN},port=9}}]"),
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
        output(true, "0", ""),
        output(true, "1", ""),
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
fn port_parse_requires_bounded_complete_field() {
    let t = "x".repeat(43);
    for (body, want) in [
        ("Bundle[{token=T port=9876}]", Some(9876)),
        ("Bundle[{port=1, token=T}]", Some(1)),
        ("Bundle[{token=T,port=65535}]", Some(65535)),
        ("result=Bundle[{token=T port=9}]", Some(9)),
        ("Bundle[{token=T port=0}]", None),
        ("Bundle[{token=T port=65536}]", None),
        ("Bundle[{token=T port=987x}]", None),
        ("Bundle[{token=T port=}]", None),
        ("Bundle[{token=T airport=9}]", None),
        ("Bundle[{token=T port=9 port=8}]", None),
        ("Bundle[{token=T}]", None),
    ] {
        let body = body.replace("token=T", &format!("token={t}"));
        let (adb, _r) = adb_with(vec![output(true, &body, "")]);
        let got = provision(&adb, "s1");
        match want {
            Some(p) => assert_eq!(got.map(|g| g.device_port).ok(), Some(p), "{body}"),
            None => assert!(got.is_err(), "{body} accepted a bad port"),
        }
    }
}

#[test]
fn disabled_physical_device_never_writes() {
    for qemu_reply in [
        output(true, "0", ""),
        output(true, "", ""),
        output(true, "qemu-1x", ""),
        output(false, "1", "getprop refused"),
    ] {
        let (adb, runner) = adb_with(vec![
            output(true, "a.b/.C", ""),
            output(true, "0", ""),
            qemu_reply,
        ]);
        let err = enable_service(&adb, "s1")
            .err()
            .map(|e| e.render())
            .unwrap_or_default();
        assert!(err.contains("Accessibility"), "{err}");
        assert!(err.contains("Agent Mobile Driver"), "{err}");
        assert!(err.contains("Allow restricted settings"), "{err}");
        let calls = runner.calls();
        assert!(
            !calls.iter().any(|c| c.contains(&"put".to_owned())),
            "physical/unproven device must see zero writes: {calls:?}"
        );
    }
}

#[test]
fn emulator_enabled_needs_no_qemu_probe() -> Result<(), Failure> {
    let existing = format!("a.b/.C:{SERVICE_COMPONENT}");
    let (adb, runner) = adb_with(vec![
        output(true, &existing, ""),
        output(true, "1", ""),
        output(true, &existing, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
    ]);
    enable_service(&adb, "s1")?;
    assert!(
        !runner
            .calls()
            .iter()
            .any(|c| c.contains(&"getprop".to_owned())),
        "enabled device must not probe qemu"
    );
    Ok(())
}

mod corrections;
