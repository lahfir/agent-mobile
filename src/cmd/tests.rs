//! `cmd` module tests: canonical-device alias precedence and lazy
//! selection discovery counting (split to satisfy the 400-line rule).

pub(super) struct TestDir(pub std::path::PathBuf);

impl TestDir {
    pub(super) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "am-cmd-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        Self(dir)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

mod canonical {
    use agent_mobile_core::error::Failure;
    use agent_mobile_core::state::{SessionEntry, StateStore};

    use super::TestDir;
    use crate::cmd::canonical_with_alias;

    #[test]
    fn live_alias_never_invokes_platform_discovery() -> Result<(), Failure> {
        let tmp = TestDir::new("canon");
        let store = StateStore::at(&tmp.0);
        std::fs::create_dir_all(store.root())?;
        let mut e = SessionEntry::new(
            "http://127.0.0.1:8770".to_owned(),
            std::process::id(),
            "t0k".to_owned(),
        );
        e.platform = Some("ios".to_owned());
        e.device_id = Some("UDID-1".to_owned());
        e.device_name = Some("iPhone 17".to_owned());
        store.upsert("ios:UDID-1", &e)?;
        let called = std::sync::atomic::AtomicBool::new(false);
        let key = canonical_with_alias("iPhone 17", &store, |_| {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            Err(Failure::local("discovery must not run", "fail"))
        })?;
        assert_eq!(key, "ios:UDID-1");
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        let state = store.load();
        assert_eq!(state.default_device_key.as_deref(), Some("ios:UDID-1"));
        assert_eq!(state.default_device.as_deref(), Some("iPhone 17"));
        Ok(())
    }
}

mod lazy {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use agent_mobile_core::error::Failure;
    use agent_mobile_core::state::{SessionEntry, StateStore};

    use super::TestDir;
    use crate::cmd::{Ctx, lazy::pick_device_with};
    use crate::platform::{PlatformDevice, PlatformScan};

    fn ctx(tag: &str) -> (Ctx, TestDir) {
        let tmp = TestDir::new(tag);
        std::fs::create_dir_all(&tmp.0).unwrap_or_default();
        (
            Ctx {
                json: false,
                app: None,
                max_depth: None,
                device: None,
                store: StateStore::at(&tmp.0),
            },
            tmp,
        )
    }

    fn ios() -> PlatformDevice {
        PlatformDevice::from_ios(agent_mobile_core::ios::Device {
            name: "iPhone 17".to_owned(),
            udid: "UDID-1".to_owned(),
            kind: "simulator",
            os: Some("26.0".to_owned()),
            state: Some("Booted".to_owned()),
        })
    }

    fn scan_once(devices: Vec<PlatformDevice>, calls: &AtomicUsize) -> PlatformScan {
        calls.fetch_add(1, Ordering::SeqCst);
        PlatformScan {
            devices,
            notes: vec![],
        }
    }

    #[test]
    fn two_stored_selectors_share_one_discovery() -> Result<(), Failure> {
        let (ctx, tmp) = ctx("scan1");
        ctx.store
            .remember_device_selection("ios:UDID-1", "iPhone 17")?;
        let calls = AtomicUsize::new(0);
        let calls_ref = &calls;
        let picked = pick_device_with(&ctx, || Ok(scan_once(vec![ios()], calls_ref)))?;
        assert_eq!(picked, "ios:UDID-1");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one scan serves all selectors"
        );
        drop(ctx);
        drop(tmp);
        Ok(())
    }

    #[test]
    fn live_alias_never_discovers() -> Result<(), Failure> {
        let (ctx, tmp) = ctx("scan0");
        let mut e = SessionEntry::new(
            "http://127.0.0.1:8770".to_owned(),
            std::process::id(),
            "t0k".to_owned(),
        );
        e.device_id = Some("UDID-1".to_owned());
        e.device_name = Some("iPhone 17".to_owned());
        ctx.store.upsert("ios:UDID-1", &e)?;
        ctx.store
            .remember_device_selection("ios:UDID-1", "iPhone 17")?;
        let calls = AtomicUsize::new(0);
        let calls_ref = &calls;
        let picked = pick_device_with(&ctx, || Ok(scan_once(vec![], calls_ref)))?;
        assert_eq!(picked, "ios:UDID-1");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "live alias must not discover"
        );
        drop(ctx);
        drop(tmp);
        Ok(())
    }

    #[test]
    fn canonical_key_heals_stale_display_name() -> Result<(), Failure> {
        let (ctx, tmp) = ctx("heal");
        ctx.store
            .remember_device_selection("ios:UDID-1", "Wrong Name")?;
        let calls = AtomicUsize::new(0);
        let calls_ref = &calls;
        let picked = pick_device_with(&ctx, || Ok(scan_once(vec![ios()], calls_ref)))?;
        assert_eq!(picked, "ios:UDID-1");
        assert_eq!(
            ctx.store.load().default_device.as_deref(),
            Some("iPhone 17"),
            "canonical key hit must also heal the display name"
        );
        drop(ctx);
        drop(tmp);
        Ok(())
    }

    #[test]
    fn empty_scan_surfaces_discovery_notes() -> Result<(), Failure> {
        let (ctx, tmp) = ctx("notes1");
        let notes = [
            "ios: simctl probe did not answer within 15000 ms",
            "android: adb unavailable",
        ];
        let result = pick_device_with(&ctx, || {
            Ok(PlatformScan {
                devices: vec![],
                notes: notes.iter().map(|n| (*n).to_owned()).collect(),
            })
        });
        match result {
            Err(Failure::Local { message, next }) => {
                assert!(
                    message.contains("no devices found"),
                    "message must name the miss: {message:?}"
                );
                for n in &notes {
                    assert!(message.contains(n), "message must carry note: {message:?}");
                }
                assert_eq!(
                    next,
                    "resolve the reported device-discovery failures, then retry"
                );
            }
            other => {
                return Err(Failure::local(
                    format!("expected miss, got: {other:?}"),
                    "fix test",
                ));
            }
        }
        drop(ctx);
        drop(tmp);
        Ok(())
    }

    #[test]
    fn empty_scan_without_notes_keeps_original_message() -> Result<(), Failure> {
        let (ctx, tmp) = ctx("notes2");
        let result = pick_device_with(&ctx, || {
            Ok(PlatformScan {
                devices: vec![],
                notes: vec![],
            })
        });
        match result {
            Err(Failure::Local { message, next }) => {
                assert_eq!(message, "no devices found");
                assert_eq!(
                    next,
                    "create a simulator with `xcrun simctl create <name> <type>`, pair a device, or create an Android AVD"
                );
            }
            other => {
                return Err(Failure::local(
                    format!("expected miss, got: {other:?}"),
                    "fix test",
                ));
            }
        }
        drop(ctx);
        drop(tmp);
        Ok(())
    }
}
