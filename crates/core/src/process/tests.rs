use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::run_bounded_until;

#[test]
fn errors_before_spawn_when_pre_cancelled() {
    let flag = AtomicBool::new(true);
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "echo must-not-run"]);
    let text = run_bounded_until(&mut cmd, Duration::from_secs(5), &flag)
        .err()
        .map(|e| e.message().to_owned())
        .unwrap_or_default();
    assert_eq!(text, "operation interrupted");
}

#[test]
fn kills_in_flight_child_fast() {
    let marker = std::env::temp_dir().join(format!("am-cancel-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let flag = Arc::new(AtomicBool::new(false));
    let (fl, mk) = (flag.clone(), marker.clone());
    let watcher = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !mk.exists() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        fl.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", &format!("touch {}; sleep 30", marker.display())]);
    let err = run_bounded_until(&mut cmd, Duration::from_secs(30), &flag);
    let _ = watcher.join();
    let _ = std::fs::remove_file(&marker);
    let text = err
        .err()
        .map(|e| e.message().to_owned())
        .unwrap_or_default();
    assert_eq!(text, "operation interrupted");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn captures_output_larger_than_pipe_capacity() {
    let mut cmd = Command::new("/bin/sh");
    cmd.args([
        "-c",
        "head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2",
    ]);
    let started = Instant::now();
    let out = run_bounded_until(&mut cmd, Duration::from_secs(30), &AtomicBool::new(false));
    let out = out.unwrap_or_else(|e| unreachable!("{}", e.render()));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(out.stdout.len(), 262_144);
    assert_eq!(out.stderr.len(), 262_144);
}

#[test]
fn cancellation_kills_process_group_and_reaps_descendants() {
    let marker = std::env::temp_dir().join(format!("am-pg-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let flag = Arc::new(AtomicBool::new(false));
    let (fl, mk) = (flag.clone(), marker.clone());
    let watcher = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !mk.exists() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        fl.store(true, Ordering::Relaxed);
    });
    let mut cmd = Command::new("/bin/sh");
    cmd.args([
        "-c",
        &format!("sleep 60 & echo $! > {}; wait", marker.display()),
    ]);
    let err = run_bounded_until(&mut cmd, Duration::from_secs(30), &flag);
    let _ = watcher.join();
    let text = err
        .err()
        .map(|e| e.message().to_owned())
        .unwrap_or_default();
    assert_eq!(text, "operation interrupted");
    let child_pid: u32 = std::fs::read_to_string(&marker)
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    let _ = std::fs::remove_file(&marker);
    assert!(child_pid > 0, "descendant pid never recorded");
    assert!(
        !super::pid_alive(child_pid),
        "descendant {child_pid} survived process-group kill"
    );
}

#[test]
fn process_identity_and_matches_pin_current_process() {
    let pid = std::process::id();
    let marker = super::process_identity(pid);
    assert!(marker.is_some(), "own process has no marker");
    assert!(super::process_matches(pid, marker.as_deref()));
    assert!(!super::process_matches(pid, Some("bogus:0")));
    assert!(
        !super::process_matches(pid, None),
        "a markerless row is stale, never live"
    );
}

#[test]
fn exited_parent_with_inherited_pipe_does_not_hang() {
    let marker = std::env::temp_dir().join(format!("am-inh-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let mut cmd = Command::new("/bin/sh");
    cmd.args([
        "-c",
        &format!("sleep 60 & echo $! > {}; exit 0", marker.display()),
    ]);
    let started = Instant::now();
    let out = run_bounded_until(&mut cmd, Duration::from_secs(30), &AtomicBool::new(false));
    let out = out.unwrap_or_else(|e| unreachable!("{}", e.render()));
    assert!(out.status.success());
    assert!(started.elapsed() < Duration::from_secs(5));
    let child_pid: u32 = std::fs::read_to_string(&marker)
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    let _ = std::fs::remove_file(&marker);
    assert!(child_pid > 0, "descendant pid never recorded");
    assert!(
        !super::pid_alive(child_pid),
        "descendant {child_pid} survived group kill after parent exit"
    );
}

#[test]
fn output_over_cap_fails_instead_of_truncating() {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "head -c 9000000 /dev/zero | tr '\\0' 'x'"]);
    let err = run_bounded_until(&mut cmd, Duration::from_secs(30), &AtomicBool::new(false))
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("stdout"), "{err}");
    assert!(err.contains("8 MiB"), "{err}");
}

#[test]
fn timeout_kills_whole_process_group() {
    let marker = std::env::temp_dir().join(format!("am-grp-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let mut cmd = Command::new("/bin/sh");
    cmd.args([
        "-c",
        &format!("sleep 60 & echo $! > {}; sleep 60", marker.display()),
    ]);
    let started = Instant::now();
    let out = run_bounded_until(
        &mut cmd,
        Duration::from_millis(400),
        &AtomicBool::new(false),
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    let err = out.err().map(|e| e.render()).unwrap_or_default();
    assert!(err.contains("did not answer"), "{err}");
    let descendant: u32 = std::fs::read_to_string(&marker)
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(descendant > 1);
    assert!(
        !super::pid_alive(descendant),
        "descendant {descendant} survived the group kill"
    );
    let _ = std::fs::remove_file(&marker);
}
