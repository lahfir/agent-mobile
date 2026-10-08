//! Pending-forward journal methods on [`StateStore`] — exact-tuple
//! records reclaimed only after the owning process is provably gone.

use crate::error::Failure;

use super::{PendingForward, StateStore};

impl StateStore {
    /// Pending-forward journal snapshot.
    #[must_use]
    pub fn pending_forwards(&self) -> Vec<PendingForward> {
        self.load().pending_forwards
    }

    /// Append the exact `serial`/`local`/`device` tuple to the pending
    /// journal, deduplicating identical records. Requires a live process
    /// marker so reclamation can prove the owner.
    ///
    /// # Errors
    /// [`Failure::Local`] when the current process has no marker or the
    /// state cannot be saved.
    pub fn record_pending_forward(
        &self,
        serial: &str,
        local_port: u16,
        device_port: u16,
    ) -> Result<(), Failure> {
        let Some(started_at) = crate::process::process_identity(std::process::id()) else {
            return Err(Failure::local(
                "cannot identify this process for the forward journal",
                "rerun the command",
            ));
        };
        let rec = PendingForward {
            owner_pid: std::process::id(),
            owner_started_at: started_at,
            serial: serial.to_owned(),
            local_port,
            device_port,
        };
        self.update(|state| {
            let dirty = !state.pending_forwards.contains(&rec);
            if dirty {
                state.pending_forwards.push(rec);
            }
            (dirty, ())
        })
    }

    /// Clear the pending record for `serial`/`local`/`device` owned by the
    /// current process — an older dead owner's identical tuple is never
    /// touched. Absent is not an error.
    ///
    /// # Errors
    /// [`Failure::Local`] when the state cannot be saved.
    pub fn clear_pending_forward(
        &self,
        serial: &str,
        local_port: u16,
        device_port: u16,
    ) -> Result<(), Failure> {
        let owner_pid = std::process::id();
        let owner_started_at = crate::process::process_identity(owner_pid).ok_or_else(|| {
            Failure::local(
                "cannot identify this process while clearing the forward journal",
                "leave the pending record in place and retry cleanup",
            )
        })?;
        self.update(|state| {
            let dirty = state
                .pending_forwards
                .iter()
                .position(|r| {
                    r.serial == serial
                        && r.local_port == local_port
                        && r.device_port == device_port
                        && r.owner_pid == owner_pid
                        && r.owner_started_at == owner_started_at
                })
                .is_some_and(|i| {
                    state.pending_forwards.remove(i);
                    true
                });
            (dirty, ())
        })
    }

    /// Remove the exact pending record; absent is not an error.
    ///
    /// # Errors
    /// [`Failure::Local`] when the state cannot be saved.
    pub fn remove_pending_forward(&self, rec: &PendingForward) -> Result<(), Failure> {
        self.update(|state| {
            let dirty = state
                .pending_forwards
                .iter()
                .position(|r| r == rec)
                .is_some_and(|i| state.pending_forwards.remove(i) == *rec);
            (dirty, ())
        })
    }
}
