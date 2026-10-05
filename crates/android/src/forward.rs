//! Owned `adb forward` lifecycle: snapshot before allocation, verify the
//! exact row, and clean up only what a failed create actually allocated.

use agent_mobile_core::error::Failure;

use crate::adb::Adb;
use crate::driver::DEVICE_PORT;

/// One parsed `forward --list` row.
pub(crate) struct ForwardRow {
    /// Owning serial.
    pub(crate) serial: String,
    /// `tcp:<port>` on the host.
    pub(crate) local: String,
    /// `tcp:<port>` on the device.
    pub(crate) remote: String,
}

/// Scoped `forward --list` parsed into rows.
pub(crate) fn list_forwards(adb: &Adb, serial: &str) -> Result<Vec<ForwardRow>, Failure> {
    let out = adb.scoped_ok(serial, "forward list", &["forward", "--list"])?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            Some(ForwardRow {
                serial: cols.next()?.to_owned(),
                local: cols.next()?.to_owned(),
                remote: cols.next()?.to_owned(),
            })
        })
        .collect())
}

/// After a failed create: remove the allocated port when known, then diff
/// pre/post rows and remove only newly-created rows matching `serial` +
/// `tcp:8770`. Preexisting and foreign rows are never touched.
fn cleanup_new_forwards(adb: &Adb, serial: &str, before: &[ForwardRow], port: Option<u16>) {
    if let Some(p) = port {
        let _ = adb.scoped(serial, &["forward", "--remove", &format!("tcp:{p}")]);
    }
    let remote = format!("tcp:{DEVICE_PORT}");
    if let Ok(after) = list_forwards(adb, serial) {
        for row in after {
            let fresh = !before
                .iter()
                .any(|b| b.serial == row.serial && b.local == row.local && b.remote == row.remote);
            if fresh && row.serial == serial && row.remote == remote {
                let _ = adb.scoped(serial, &["forward", "--remove", &row.local]);
            }
        }
    }
}

/// `adb -s <serial> forward tcp:0 tcp:8770` with a pre-snapshot: verify the
/// exact row persists, and on any verification failure remove only what
/// this call allocated.
///
/// # Errors
/// [`Failure::Local`] when allocation, listing, or verification fails; any
/// allocated forward is removed first.
pub(crate) fn create_forward(adb: &Adb, serial: &str) -> Result<u16, Failure> {
    let before = list_forwards(adb, serial)?;
    let out = adb.scoped_ok(
        serial,
        "forward",
        &["forward", "tcp:0", &format!("tcp:{DEVICE_PORT}")],
    )?;
    let Ok(port) = out.stdout.trim().parse::<u16>() else {
        cleanup_new_forwards(adb, serial, &before, None);
        return Err(Failure::local(
            format!("adb forward returned unparseable output on {serial}"),
            "retry the forward",
        ));
    };
    let remote = format!("tcp:{DEVICE_PORT}");
    let local = format!("tcp:{port}");
    let found = match list_forwards(adb, serial) {
        Ok(rows) => rows
            .iter()
            .any(|r| r.serial == serial && r.local == local && r.remote == remote),
        Err(e) => {
            cleanup_new_forwards(adb, serial, &before, Some(port));
            return Err(e);
        }
    };
    if found {
        Ok(port)
    } else {
        cleanup_new_forwards(adb, serial, &before, Some(port));
        Err(Failure::local(
            format!("forward tcp:{port} on {serial} did not persist"),
            "retry `adb -s {serial} forward tcp:0 tcp:{DEVICE_PORT}` manually",
        ))
    }
}

/// Remove exactly `adb -s <serial> forward --remove tcp:<port>`.
///
/// # Errors
/// Propagates the runner/adb failure.
pub(crate) fn remove_forward(adb: &Adb, serial: &str, port: u16) -> Result<(), Failure> {
    adb.scoped_ok(
        serial,
        "forward remove",
        &["forward", "--remove", &format!("tcp:{port}")],
    )
    .map(|_| ())
}
