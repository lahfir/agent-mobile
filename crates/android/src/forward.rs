//! Owned `adb forward` lifecycle: snapshot before allocation, verify the
//! exact row, and clean up only what a failed create actually allocated.

use agent_mobile_core::error::Failure;

use std::net::TcpListener;

use crate::adb::Adb;

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
    out.stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() != 3 {
                return Err(Failure::local(
                    format!(
                        "malformed `adb forward --list` row on {serial}: {}",
                        crate::adb::bounded_diagnostic(line)
                    ),
                    "inspect `adb forward --list` output and retry",
                ));
            }
            Ok(ForwardRow {
                serial: cols[0].to_owned(),
                local: cols[1].to_owned(),
                remote: cols[2].to_owned(),
            })
        })
        .collect()
}

/// Records and clears in-flight forward tuples so a crashed session's
/// exact `serial`/`local`/`device` row can be reclaimed later.
pub trait ForwardJournal: Send + Sync {
    /// Persist the pending tuple before the ADB side effect.
    ///
    /// # Errors
    /// [`Failure::Local`] when the journal write fails.
    fn record(&self, serial: &str, local_port: u16, device_port: u16) -> Result<(), Failure>;
    /// Drop the pending tuple after the forward is removed or promoted.
    ///
    /// # Errors
    /// [`Failure::Local`] when the journal write fails.
    fn clear(&self, serial: &str, local_port: u16, device_port: u16) -> Result<(), Failure>;
}

/// Claim an ephemeral host port by binding and releasing `127.0.0.1:0` —
/// the ADB create is then issued with `--no-rebind` on that exact port.
///
/// # Errors
/// [`Failure::Local`] when no port can be bound.
pub(crate) fn reserve_local_port() -> Result<u16, Failure> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(Failure::from)
}

/// After a failed create: relist on an uncancelled clone and remove only
/// newly-created rows matching `serial` + the session's
/// `tcp:<device_port>` remote — each removal re-verified through
/// [`remove_owned_forward`]. Preexisting and foreign rows are never
/// touched; a foreign remote occupying an allocated local port is left
/// alone.
///
/// # Errors
/// [`Failure::Local`] when listing or a proven-owned removal fails.
fn cleanup_new_forwards(
    adb: &Adb,
    serial: &str,
    before: &[ForwardRow],
    device_port: u16,
) -> Result<(), Failure> {
    let cleanup_adb = adb.without_cancellation();
    let remote = format!("tcp:{device_port}");
    let after = list_forwards(&cleanup_adb, serial)?;
    for row in after {
        let fresh = !before
            .iter()
            .any(|b| b.serial == row.serial && b.local == row.local && b.remote == row.remote);
        if !(fresh && row.serial == serial && row.remote == remote) {
            continue;
        }
        let Some(local) = row
            .local
            .strip_prefix("tcp:")
            .and_then(|p| p.parse::<u16>().ok())
        else {
            return Err(Failure::local(
                format!(
                    "fresh forward {} {} {} on {serial} has a non-tcp local port",
                    row.serial, row.local, row.remote
                ),
                format!("inspect `adb -s {serial} forward --list`"),
            ));
        };
        remove_owned_forward(&cleanup_adb, serial, local, device_port)?;
    }
    Ok(())
}

/// `adb -s <serial> forward --no-rebind tcp:<local_port> tcp:<device_port>`
/// with a pre-snapshot:
/// verify the exact row persists, and on any failure — including a
/// cancelled or errored create that may have raced a real allocation —
/// roll back only the fresh rows this call created.
///
/// # Errors
/// [`Failure::Local`] on allocation, listing, or verification failure; a
/// rollback failure is composed into the returned error.
pub(crate) fn create_forward(
    adb: &Adb,
    serial: &str,
    local_port: u16,
    device_port: u16,
) -> Result<u16, Failure> {
    let before = list_forwards(adb, serial)?;
    if let Err(e) = adb.scoped_ok(
        serial,
        "forward",
        &[
            "forward",
            "--no-rebind",
            &format!("tcp:{local_port}"),
            &format!("tcp:{device_port}"),
        ],
    ) {
        return Err(rollback_or(
            e,
            cleanup_new_forwards(adb, serial, &before, device_port),
            serial,
            None,
            device_port,
        ));
    }
    let port = local_port;
    let remote = format!("tcp:{device_port}");
    let local = format!("tcp:{port}");
    let found = match list_forwards(adb, serial) {
        Ok(rows) => rows
            .iter()
            .any(|r| r.serial == serial && r.local == local && r.remote == remote),
        Err(e) => {
            return Err(rollback_or(
                e,
                cleanup_new_forwards(adb, serial, &before, device_port),
                serial,
                Some(port),
                device_port,
            ));
        }
    };
    if found {
        Ok(port)
    } else {
        Err(rollback_or(
            Failure::local(
                format!("forward tcp:{port} on {serial} did not persist"),
                format!(
                    "retry `adb -s {serial} forward --no-rebind tcp:{local_port} tcp:{device_port}`"
                ),
            ),
            cleanup_new_forwards(adb, serial, &before, device_port),
            serial,
            Some(port),
            device_port,
        ))
    }
}

/// Compose a startup error with its rollback result — a failed rollback
/// is never dropped silently and always carries the qualified manual
/// remedy for the known serial/local/remote tuple.
fn rollback_or(
    cause: Failure,
    rollback: Result<(), Failure>,
    serial: &str,
    known_local: Option<u16>,
    device_port: u16,
) -> Failure {
    let Err(cleanup) = rollback else {
        return cause;
    };
    let remedy = known_local.map_or_else(
        || {
            format!(
                "inspect `adb -s {serial} forward --list` for rows with remote tcp:{device_port}"
            )
        },
        |local| {
            format!(
                "confirm remote tcp:{device_port}, then `adb -s {serial} forward --remove tcp:{local}`"
            )
        },
    );
    Failure::local(
        format!(
            "{}; forward cleanup also failed: {}{}",
            cause.message(),
            cleanup.message(),
            known_local.map_or_else(String::new, |l| {
                format!(" (owned row: {serial} tcp:{l} tcp:{device_port})")
            }),
        ),
        remedy,
    )
}

/// Remove exactly `adb -s <serial> forward --remove tcp:<local>` for the
/// row `<serial> tcp:<local> tcp:<device_port>` — fail-closed ownership:
///
/// - no exact row → success without issuing `remove` (a different remote
///   on the same local port is foreign and untouched);
/// - exact row → remove, then relist; success only when it is gone.
///
/// A same-remote replacement race remains an adb limitation this cannot
/// distinguish.
///
/// # Errors
/// [`Failure::Local`] when listing fails or the exact row survives.
pub(crate) fn remove_owned_forward(
    adb: &Adb,
    serial: &str,
    local_port: u16,
    device_port: u16,
) -> Result<(), Failure> {
    let local = format!("tcp:{local_port}");
    let remote = format!("tcp:{device_port}");
    let rows = list_forwards(adb, serial)?;
    let owned = rows
        .iter()
        .any(|r| r.serial == serial && r.local == local && r.remote == remote);
    if !owned {
        return Ok(());
    }
    if let Err(e) = adb.scoped_ok(serial, "forward remove", &["forward", "--remove", &local]) {
        let gone = list_forwards(adb, serial).map(|rows| {
            !rows
                .iter()
                .any(|r| r.serial == serial && r.local == local && r.remote == remote)
        })?;
        return if gone { Ok(()) } else { Err(e) };
    }
    let still = list_forwards(adb, serial)?
        .iter()
        .any(|r| r.serial == serial && r.local == local && r.remote == remote);
    if still {
        return Err(Failure::local(
            format!("forward {local} on {serial} survived removal"),
            format!("confirm remote {remote}, then `adb -s {serial} forward --remove {local}`"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
