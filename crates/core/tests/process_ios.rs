//! U8 coverage: device URL/command construction, token minting, pid and TCP
//! liveness, the atomic boot lock, and the supervised child's log replay and
//! drop-guard reaping.

use std::ffi::OsStr;
use std::io::Read as _;
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

use agent_mobile_core::error::Failure;
use agent_mobile_core::ios::{self, Device, DriverSource};
use agent_mobile_core::process::{self, BootLock, ServeChild};

type TestResult = Result<(), Failure>;

fn sim() -> Device {
    Device {
        name: "iPhone 17 Pro Max".to_owned(),
        udid: "9B7CEE9E-0000-4000-8000-0123456789AB".to_owned(),
        kind: "simulator",
        os: Some("iOS 26.0".to_owned()),
        state: Some("Shutdown".to_owned()),
    }
}

fn phone() -> Device {
    Device {
        name: "Lahfir's iPhone".to_owned(),
        udid: "BD631C44-37F1-5897-975D-C0250525B017".to_owned(),
        kind: "device",
        os: Some("iOS 27.0".to_owned()),
        state: Some("available".to_owned()),
    }
}

fn tmp(tag: &str) -> Result<std::path::PathBuf, Failure> {
    let dir = std::env::temp_dir().join(format!(
        "am-u8-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn env_of(cmd: &Command, key: &str) -> Option<String> {
    cmd.get_envs()
        .find(|(k, _)| *k == OsStr::new(key))
        .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
}

/// Poll `child`'s fresh log output for `needle` until `budget` expires.
fn await_output(child: &mut ServeChild, needle: &str, budget: Duration) -> String {
    let deadline = Instant::now() + budget;
    let mut out = String::new();
    while !out.contains(needle) && Instant::now() < deadline {
        out.push_str(&child.new_output());
        std::thread::sleep(Duration::from_millis(25));
    }
    out
}

#[test]
fn token_mints_24_hex_and_never_repeats() -> TestResult {
    let a = process::mint_token()?;
    let b = process::mint_token()?;
    assert_eq!(a.len(), 24);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
    Ok(())
}

#[test]
fn bonjour_host_drops_apostrophes_and_dashes_spaces() {
    assert_eq!(ios::bonjour_host("Lahfir's iPhone"), "Lahfirs-iPhone.local");
    assert_eq!(ios::bonjour_host("My Phone"), "My-Phone.local");
    assert_eq!(ios::bonjour_host("plain"), "plain.local");
}

#[test]
fn driver_url_loopback_for_sim_bonjour_for_device() {
    assert_eq!(ios::driver_url(&sim(), 8770), "http://127.0.0.1:8770");
    assert_eq!(
        ios::driver_url(&phone(), 8770),
        "http://Lahfirs-iPhone.local:8770"
    );
}

#[test]
fn serve_command_sim_args_and_runner_env() -> TestResult {
    let dir = tmp("cmd-sim")?;
    let cmd = ios::serve_command(&sim(), 8770, "tok123", &DriverSource::Project(dir))?;
    assert_eq!(cmd.get_program(), OsStr::new("xcodebuild"));
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    assert!(joined.contains("test -project ToDo.xcodeproj -scheme ToDo"));
    assert!(joined.contains("platform=iOS Simulator,id=9B7CEE9E-0000-4000-8000-0123456789AB"));
    assert!(joined.contains("-only-testing:ToDoUITests/AgentMobileServer/testServe"));
    assert!(joined.contains("CODE_SIGNING_ALLOWED=NO"));
    assert_eq!(
        env_of(&cmd, "TEST_RUNNER_AGENT_MOBILE_PORT").as_deref(),
        Some("8770")
    );
    assert_eq!(
        env_of(&cmd, "TEST_RUNNER_AGENT_MOBILE_TOKEN").as_deref(),
        Some("tok123")
    );
    assert!(env_of(&cmd, "TEST_RUNNER_AGENT_MOBILE_BIND").is_none());
    Ok(())
}

#[test]
fn serve_command_device_binds_wildcard() -> TestResult {
    let dir = tmp("cmd-dev")?;
    let cmd = ios::serve_command(&phone(), 8770, "t", &DriverSource::Project(dir))?;
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    assert!(joined.contains("platform=iOS,id=BD631C44-37F1-5897-975D-C0250525B017"));
    assert!(joined.contains("-allowProvisioningUpdates"));
    assert_eq!(
        env_of(&cmd, "TEST_RUNNER_AGENT_MOBILE_BIND").as_deref(),
        Some("0.0.0.0")
    );
    Ok(())
}

/// Running inside the workspace, the ancestor walk from crates/core must
/// land on the repo's fixtures/driver project.
#[test]
fn driver_source_finds_the_checkout_layout() -> TestResult {
    match ios::driver_source()? {
        DriverSource::Project(dir) => {
            assert!(dir.join("ToDo.xcodeproj").exists());
            assert!(dir.ends_with("fixtures/driver"));
        }
        DriverSource::Prebuilt { .. } => {
            return Err(Failure::local(
                "expected the project source",
                "check probes",
            ));
        }
    }
    Ok(())
}

#[test]
fn prebuilt_runner_uses_test_without_building() -> TestResult {
    let dir = tmp("prebuilt")?;
    let xctestrun = dir.join("ToDo_ToDo_iphonesimulator26.0-arm64.xctestrun");
    std::fs::write(&xctestrun, "<plist/>")?;
    let cmd = ios::serve_command(
        &sim(),
        8770,
        "tok",
        &DriverSource::Prebuilt {
            dir: dir.clone(),
            xctestrun: xctestrun.clone(),
        },
    )?;
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args.first().map(String::as_str),
        Some("test-without-building")
    );
    assert!(args.iter().any(|a| a == "-xctestrun"));
    assert!(args.contains(&xctestrun.to_string_lossy().into_owned()));
    assert_eq!(
        env_of(&cmd, "TEST_RUNNER_AGENT_MOBILE_TOKEN").as_deref(),
        Some("tok")
    );
    Ok(())
}

#[test]
fn prebuilt_runner_refuses_physical_device() -> TestResult {
    let dir = tmp("prebuilt-dev")?;
    let xctestrun = dir.join("r.xctestrun");
    std::fs::write(&xctestrun, "<plist/>")?;
    let err = ios::serve_command(
        &phone(),
        8770,
        "t",
        &DriverSource::Prebuilt { dir, xctestrun },
    );
    match err {
        Err(f) => {
            let text = f.render();
            assert!(text.contains("simulator-only"), "{text}");
        }
        Ok(_) => {
            return Err(Failure::local(
                "physical+prebuilt must fail",
                "check source",
            ));
        }
    }
    Ok(())
}

#[test]
fn pid_alive_self_true() {
    assert!(process::pid_alive(std::process::id()));
}

#[test]
fn pid_dead_child_reports_false() -> TestResult {
    let mut child = Command::new("true").spawn()?;
    let pid = child.id();
    child.wait()?;
    assert!(!process::pid_alive(pid));
    Ok(())
}

#[test]
fn tcp_ready_tracks_a_bound_listener() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?.to_string();
    assert!(process::tcp_ready(&addr));
    assert!(
        !process::tcp_ready("127.0.0.1:1"),
        "port 1 must refuse; a freed ephemeral port can be reclaimed by a parallel test"
    );
    Ok(())
}

#[test]
fn wait_tcp_times_out_on_dead_port() {
    let start = Instant::now();
    let ready = process::wait_tcp(
        "127.0.0.1:1",
        Duration::from_millis(120),
        Duration::from_millis(40),
    );
    assert!(!ready);
    assert!(start.elapsed() >= Duration::from_millis(100));
}

#[test]
fn boot_lock_is_atomic_and_drops() -> TestResult {
    let dir = tmp("bootlock")?;
    let path = dir.join("boot.lock");
    {
        let first = BootLock::take(&path)?;
        assert!(first.is_some(), "first take must win");
        assert!(
            BootLock::take(&path)?.is_none(),
            "held lock blocks a second"
        );
        assert!(BootLock::age(&path).is_some());
    }
    assert!(BootLock::take(&path)?.is_some(), "drop releases the lock");
    BootLock::clear(&path);
    assert!(BootLock::age(&path).is_none());
    Ok(())
}

#[test]
fn serve_child_replays_log_and_drop_reaps() -> TestResult {
    let dir = tmp("child")?;
    let log = dir.join("child.log");
    let pid;
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hello-log; sleep 30"]);
        let mut child = ServeChild::spawn_logged(&mut cmd, &log)?;
        pid = child.pid();
        let out = await_output(&mut child, "hello-log", Duration::from_secs(2));
        assert!(out.contains("hello-log"), "{out:?}");
    }
    assert!(!process::pid_alive(pid));
    let mut persisted = String::new();
    std::fs::File::open(&log)?.read_to_string(&mut persisted)?;
    assert!(persisted.contains("hello-log"));
    Ok(())
}

#[test]
fn serve_child_log_appends_across_runs() -> TestResult {
    let dir = tmp("logappend")?;
    let log = dir.join("driver.log");
    for _ in 0..2 {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo line"]);
        let mut child = ServeChild::spawn_logged(&mut cmd, &log)?;
        let _ = child.wait();
    }
    let text = std::fs::read_to_string(&log)?;
    assert_eq!(text.matches("line").count(), 2);
    Ok(())
}

#[test]
fn token_file_names_sanitize() {
    assert_eq!(
        agent_mobile_core::state::StateStore::token_file_for("Lahfir's iPhone"),
        "lahfir-s-iphone"
    );
    assert_eq!(
        agent_mobile_core::state::StateStore::token_file_for("iPhone 17 Pro Max"),
        "iphone-17-pro-max"
    );
}
