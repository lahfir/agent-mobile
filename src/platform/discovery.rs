//! Combined discovery: the iOS `simctl`/`devicectl` scan and the Android
//! `adb`/AVD scan probe concurrently and each fails independently — one
//! missing toolchain degrades to a prefixed note, never kills the list.

use agent_mobile_android::AndroidAdapter;
use agent_mobile_core::error::Failure;
use agent_mobile_core::ios;

use super::{PlatformDevice, PlatformScan};

/// Probe both platforms at once. One side failing contributes only its
/// `ios:`/`android:` note; both failing is a single local failure naming
/// each toolchain's remedy.
///
/// # Errors
/// [`Failure::Local`] when both sides fail.
pub fn discover() -> Result<PlatformScan, Failure> {
    std::thread::scope(|scope| {
        let ios_probe = scope.spawn(ios::list_devices);
        let android_probe =
            scope.spawn(|| AndroidAdapter::from_environment().and_then(|a| a.discover()));
        let ios = join_probe(ios_probe, "ios");
        let android = join_probe(android_probe, "android");
        combine(ios, android)
    })
}

/// Merge both probe results: both failing is one [`Failure::Local`] with
/// both bounded reasons and both remedies; one side's failure degrades to
/// a prefixed note; successes contribute devices plus their own notes.
pub(crate) fn combine(
    ios_r: Result<ios::DeviceScan, Failure>,
    android_r: Result<agent_mobile_android::AndroidScan, Failure>,
) -> Result<PlatformScan, Failure> {
    if let (Err(i), Err(a)) = (&ios_r, &android_r) {
        return Err(Failure::local(
            format!(
                "device discovery failed — ios: {}; android: {}",
                reason(i),
                reason(a)
            ),
            "install Xcode for iOS, or run `scripts/setup-android-sdk.sh --check` for Android",
        ));
    }
    let (devices, notes) = collect(ios_r, android_r);
    Ok(PlatformScan { devices, notes })
}

/// Join a probe thread; a panic degrades to a failed side.
fn join_probe<T>(
    handle: std::thread::ScopedJoinHandle<'_, Result<T, Failure>>,
    side: &str,
) -> Result<T, Failure> {
    handle.join().unwrap_or_else(|_| {
        Err(Failure::local(
            format!("{side} discovery panicked"),
            "report a bug",
        ))
    })
}

/// Merge both results into devices + prefixed notes.
pub(crate) fn collect(
    ios_r: Result<ios::DeviceScan, Failure>,
    android_r: Result<agent_mobile_android::AndroidScan, Failure>,
) -> (Vec<PlatformDevice>, Vec<String>) {
    let mut devices = Vec::new();
    let mut notes = Vec::new();
    match ios_r {
        Ok(scan) => {
            notes.extend(scan.notes.iter().map(|n| format!("ios: {n}")));
            devices.extend(scan.devices.into_iter().map(PlatformDevice::from_ios));
        }
        Err(e) => notes.push(format!("ios: {}", reason(&e))),
    }
    match android_r {
        Ok(scan) => {
            notes.extend(scan.notes.iter().map(|n| format!("android: {n}")));
            devices.extend(scan.targets.into_iter().map(PlatformDevice::from_android));
        }
        Err(e) => notes.push(format!("android: {}", reason(&e))),
    }
    (devices, notes)
}

/// A failure rendered bounded and control-free — probe text never carries
/// secrets anyway, but the combined error must stay one clean line.
fn reason(f: &Failure) -> String {
    let text = f.render();
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(300)
        .collect();
    clean.trim().to_owned()
}
