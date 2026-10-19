//! `devices --platform` probes only the named platform, so a filtered call
//! never waits on the other platform's tools.

#[allow(dead_code, reason = "shared harness; this suite uses the unwired half")]
mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use agent_mobile_core::error::Failure;

use common::{code, run, stderr, tmp_home};

/// An SDK whose `adb` logs each call to `adb.log` and lists no devices.
fn fake_sdk(home: &Path) -> Result<PathBuf, Failure> {
    let sdk = home.join("sdk");
    for (tool, body) in [
        (
            "platform-tools/adb",
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$1\" = devices ] && echo 'List of devices attached'\nexit 0\n",
                home.join("adb.log").display()
            ),
        ),
        ("emulator/emulator", "#!/bin/sh\nexit 0\n".to_owned()),
    ] {
        let path = sdk.join(tool);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, body)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(sdk)
}

#[test]
fn platform_filter_skips_the_other_platform() -> Result<(), Failure> {
    let home = tmp_home("devices-platform")?;
    let sdk = fake_sdk(&home)?;
    let sdk = sdk.to_string_lossy();
    let envs = [("ANDROID_HOME", &*sdk), ("ANDROID_SDK_ROOT", &*sdk)];
    run(&["devices", "--platform", "ios"], &home, &envs)?;
    assert!(
        !home.join("adb.log").exists(),
        "an iOS-only listing ran adb"
    );
    let out = run(&["devices", "--platform", "android"], &home, &envs)?;
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(home.join("adb.log").exists(), "the fake SDK was not used");
    Ok(())
}

/// A platform filter whose only probe fails is an error with that
/// platform's remedy, not an empty list.
#[test]
fn filtered_platform_failure_is_an_error() -> Result<(), Failure> {
    let home = tmp_home("devices-no-sdk")?;
    let empty = home.to_string_lossy();
    let envs = [
        ("ANDROID_HOME", &*empty),
        ("ANDROID_SDK_ROOT", &*empty),
        ("PATH", "/usr/bin:/bin"),
    ];
    let out = run(&["devices", "--platform", "android"], &home, &envs)?;
    assert_ne!(
        code(&out),
        0,
        "a missing SDK listed no devices and succeeded"
    );
    assert!(stderr(&out).contains("adb"), "{}", stderr(&out));
    Ok(())
}

/// A platform-keyed `--device` resolves on that platform only.
#[test]
fn ios_device_key_never_probes_android() -> Result<(), Failure> {
    let home = tmp_home("device-key-ios")?;
    let sdk = fake_sdk(&home)?;
    let sdk = sdk.to_string_lossy();
    let envs = [("ANDROID_HOME", &*sdk), ("ANDROID_SDK_ROOT", &*sdk)];
    let key = "ios:00000000-0000-0000-0000-000000000000";
    let out = run(&["--device", key, "status"], &home, &envs)?;
    assert_ne!(code(&out), 0, "an unknown device answered status");
    assert!(
        !home.join("adb.log").exists(),
        "an iOS key ran adb: {}",
        stderr(&out)
    );
    Ok(())
}

/// An `android:` or `avd:` key never runs the iOS toolchain.
#[test]
fn android_device_keys_never_probe_ios() -> Result<(), Failure> {
    for key in ["android:avd:nope", "avd:nope"] {
        let home = tmp_home("device-key-android")?;
        let sdk = fake_sdk(&home)?;
        let bin = home.join("fakebin");
        std::fs::create_dir_all(&bin)?;
        let xcrun = bin.join("xcrun");
        let log = home.join("xcrun.log");
        std::fs::write(
            &xcrun,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit 1\n", log.display()),
        )?;
        std::fs::set_permissions(&xcrun, std::fs::Permissions::from_mode(0o755))?;
        let (sdk, path) = (
            sdk.to_string_lossy(),
            format!("{}:/usr/bin:/bin", bin.display()),
        );
        let envs = [
            ("ANDROID_HOME", &*sdk),
            ("ANDROID_SDK_ROOT", &*sdk),
            ("PATH", path.as_str()),
        ];
        let out = run(&["--device", key, "status"], &home, &envs)?;
        assert_ne!(code(&out), 0, "{key}: an unknown device answered status");
        assert!(!log.exists(), "{key} ran xcrun: {}", stderr(&out));
        assert!(home.join("adb.log").exists(), "{key} never probed Android");
    }
    Ok(())
}
