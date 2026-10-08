use std::path::PathBuf;

use agent_mobile_core::error::Failure;

use crate::adb::{Adb, CommandOutput, CommandRunner};
use crate::session::AndroidAdapter;
use crate::session::testkit::*;
use crate::testkit::{FakeRunner, output};

#[test]
fn cancel_after_forward_removes_exact_row_without_probe() -> Result<(), Failure> {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct FlipRunner {
        inner: std::sync::Arc<FakeRunner>,
        flag: std::sync::Arc<AtomicBool>,
        lists: std::sync::atomic::AtomicU32,
    }
    impl CommandRunner for FlipRunner {
        fn run(
            &self,
            program: &std::path::Path,
            args: &[&str],
            timeout: std::time::Duration,
        ) -> Result<CommandOutput, Failure> {
            if args.contains(&"--list") && self.lists.fetch_add(1, Ordering::Relaxed) == 1 {
                self.flag.store(true, Ordering::Relaxed);
            }
            self.inner.run(program, args, timeout)
        }
    }
    let flag = std::sync::Arc::new(AtomicBool::new(false));
    let mut replies = vec![
        output(true, "device\n", ""),
        output(true, "Success\n", ""),
        output(true, SVC, ""),
        output(true, "1", ""),
        output(true, SVC, ""),
        output(
            true,
            "Bound services:{Service[label=Agent Mobile Driver, feedbackType[0]]}",
            "",
        ),
        output(
            true,
            &format!("result=Bundle[{{token={TOKEN} port=9876}}]"),
            "",
        ),
        output(true, "other tcp:1 tcp:2", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2\ns1 tcp:{LOCAL} tcp:9876", ""),
        output(true, "other tcp:1 tcp:2\ns1 tcp:{LOCAL} tcp:9876", ""),
        output(true, "", ""),
        output(true, "other tcp:1 tcp:2", ""),
    ];
    let inner = FakeRunner::scripted(std::mem::take(&mut replies));
    let runner = std::sync::Arc::new(FlipRunner {
        inner,
        flag: flag.clone(),
        lists: std::sync::atomic::AtomicU32::new(0),
    });
    let apk = apk_fixture()?;
    let adapter = AndroidAdapter::for_test(
        Adb::with_runner(PathBuf::from("adb"), runner.clone()),
        PathBuf::from("emulator"),
        Some(apk),
    );
    let err = start_test_session_until(&adapter, "s1", flag)
        .err()
        .map(|e| e.message().to_owned())
        .unwrap_or_default();
    assert_eq!(err, "operation interrupted", "{err}");
    let calls = runner.inner.calls();
    let fwd = reserved_port(&calls);
    let remove = calls.iter().rfind(|c| c.contains(&"--remove".to_owned()));
    let remove = remove.ok_or_else(|| Failure::local("forward never removed", "fail"))?;
    assert_eq!(
        remove,
        &["-s", "s1", "forward", "--remove", &format!("tcp:{fwd}")]
    );
    Ok(())
}

const SNAPSHOT_BODY: &str = "{\"version\":\"1\",\"ok\":true,\"command\":\"snapshot\",\"data\":{\"app\":\"com.x\",\"snapshot_id\":\"a1\",\"ref_count\":0,\"complete\":true,\"settled\":true,\"reads\":1,\"text\":\"\",\"tree\":{\"role\":\"g\",\"name\":\"\",\"value\":\"\",\"ref_id\":\"@a1:e1\",\"states\":[],\"available_actions\":[],\"bounds\":{\"x\":0.0,\"y\":0.0,\"width\":1.0,\"height\":1.0},\"children\":[]}}}";

#[test]
fn probe_rejects_non_status_envelope_and_rolls_back() -> Result<(), Failure> {
    let (adapter, runner) = adapter_with_body(
        vec![
            output(true, "other tcp:1 tcp:2\ns1 tcp:{LOCAL} tcp:9876", ""),
            output(true, "", ""),
            output(true, "other tcp:1 tcp:2", ""),
        ],
        SNAPSHOT_BODY,
    )?;
    let err = start_test_session(&adapter, "s1")
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("probe"), "{err}");
    let calls = runner.calls();
    let fwd = reserved_port(&calls);
    let remove = calls.iter().rfind(|c| c.contains(&"--remove".to_owned()));
    let remove = remove.ok_or_else(|| Failure::local("forward never removed", "fail"))?;
    assert_eq!(
        remove,
        &["-s", "s1", "forward", "--remove", &format!("tcp:{fwd}")]
    );
    Ok(())
}

struct RecordingJournal {
    inner: std::sync::Arc<FakeRunner>,
    order: std::sync::Mutex<Vec<String>>,
    fail_clear: std::sync::atomic::AtomicBool,
}

impl crate::forward::ForwardJournal for RecordingJournal {
    fn record(&self, serial: &str, local_port: u16, device_port: u16) -> Result<(), Failure> {
        let seen = self
            .inner
            .calls()
            .iter()
            .any(|c| c.iter().any(|a| a == "--no-rebind"));
        let mut o = self
            .order
            .lock()
            .map_err(|_| Failure::local("poisoned", "x"))?;
        o.push(format!(
            "record:{serial}:{local_port}:{device_port}:forward_seen={seen}"
        ));
        Ok(())
    }

    fn clear(&self, serial: &str, local_port: u16, device_port: u16) -> Result<(), Failure> {
        if self.fail_clear.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Failure::local("journal clear refused", "retry"));
        }
        let dbg = format!("{:?}", self.inner.calls());
        let removed = self.inner.calls().iter().any(|c| {
            c.iter().any(|a| a == "--remove") && c.iter().any(|a| *a == format!("tcp:{local_port}"))
        });
        let mut o = self
            .order
            .lock()
            .map_err(|_| Failure::local("poisoned", "x"))?;
        o.push(format!(
            "clear:{serial}:{local_port}:{device_port}:row_removed={removed} {dbg}"
        ));
        Ok(())
    }
}

#[test]
fn journal_records_before_forward_and_clears_after_commit() -> Result<(), Failure> {
    let (adapter, runner, journal) = {
        let (adapter, runner) = adapter_with()?;
        let journal = std::sync::Arc::new(RecordingJournal {
            inner: runner.clone(),
            order: std::sync::Mutex::new(Vec::new()),
            fail_clear: std::sync::atomic::AtomicBool::new(false),
        });
        (adapter, runner, journal)
    };
    let mut session = adapter.start_session_until_journaled(
        "s1",
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        journal.clone(),
    )?;
    let calls = runner.calls();
    let fwd = reserved_port(&calls);
    assert_ne!(fwd, 0);
    {
        let order = journal.order.lock().unwrap_or_else(|_| unreachable!());
        assert!(
            order[0].starts_with(&format!("record:s1:{fwd}:9876:forward_seen=false")),
            "journal must precede the forward call: {order:?}"
        );
    }
    session.commit_forward_journal()?;
    {
        let order = journal.order.lock().unwrap_or_else(|_| unreachable!());
        assert!(
            order[1].contains("row_removed=false"),
            "commit clears while the row is still owned: {order:?}"
        );
    }
    session.close()?;
    Ok(())
}

#[test]
fn close_without_commit_removes_row_then_clears_journal() -> Result<(), Failure> {
    let (adapter, runner) = adapter_with()?;
    let journal = std::sync::Arc::new(RecordingJournal {
        inner: runner,
        order: std::sync::Mutex::new(Vec::new()),
        fail_clear: std::sync::atomic::AtomicBool::new(false),
    });
    let session = adapter.start_session_until_journaled(
        "s1",
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        journal.clone(),
    )?;
    session.close()?;
    let order = journal.order.lock().unwrap_or_else(|_| unreachable!());
    assert!(
        order.last().is_some_and(|c| c.contains("row_removed=true")),
        "close must remove the row before clearing: {order:?}"
    );
    Ok(())
}

#[test]
fn journal_clear_failure_is_surfaced_and_retained() -> Result<(), Failure> {
    let (adapter, _runner) = adapter_with()?;
    let journal = std::sync::Arc::new(RecordingJournal {
        inner: FakeRunner::scripted(Vec::new()),
        order: std::sync::Mutex::new(Vec::new()),
        fail_clear: std::sync::atomic::AtomicBool::new(true),
    });
    let mut session = adapter.start_session_until_journaled(
        "s1",
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        journal,
    )?;
    let err = session
        .commit_forward_journal()
        .err()
        .map(|e| e.message().to_owned())
        .unwrap_or_default();
    assert!(err.contains("journal clear refused"), "{err}");
    Ok(())
}

#[test]
fn failure_after_alloc_removes_row_then_clears_journal() -> Result<(), Failure> {
    let (adapter, runner) = adapter_with_body(
        vec![
            output(true, "other tcp:1 tcp:2\ns1 tcp:{LOCAL} tcp:9876", ""),
            output(true, "", ""),
            output(true, "other tcp:1 tcp:2", ""),
        ],
        SNAPSHOT_BODY,
    )?;
    let journal = std::sync::Arc::new(RecordingJournal {
        inner: runner,
        order: std::sync::Mutex::new(Vec::new()),
        fail_clear: std::sync::atomic::AtomicBool::new(false),
    });
    let err = adapter
        .start_session_until_journaled(
            "s1",
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            journal.clone(),
        )
        .err()
        .map(|e| e.render())
        .unwrap_or_default();
    assert!(err.contains("probe"), "{err}");
    let order = journal.order.lock().unwrap_or_else(|_| unreachable!());
    assert!(
        order[0].contains("forward_seen=false"),
        "record before forward: {order:?}"
    );
    assert!(
        order.last().is_some_and(|c| c.contains("row_removed=true")),
        "clear must run after the row is removed: {order:?}"
    );
    Ok(())
}
