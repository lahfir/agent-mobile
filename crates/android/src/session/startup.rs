//! Startup sequence: provision → reserve → journal → forward → probe →
//! bridge, journaling the exact tuple before the ADB side effect.

use std::path::PathBuf;
use std::sync::Arc;

use agent_mobile_core::error::Failure;

use crate::adb::Adb;
use crate::driver::{Provisioned, enable_service, ensure_apk, install, probe_status, provision};
use crate::forward::{self, ForwardJournal, create_forward, remove_owned_forward};
use crate::http::start_bridge;
use crate::lifecycle::AdbLifecycle;
use crate::session::{AndroidAdapter, AndroidSession, check_device_state};

impl AndroidAdapter {
    /// Ordered session bring-up for `serial`: device check → APK →
    /// install → enable/bind service → provision (rotates token onto a
    /// fresh ephemeral port) → forward to that port → probe → bridge.
    /// Each later step's failure unwinds what this call created. The
    /// exact forward tuple is journaled before the ADB side effect, so
    /// a crashed startup can be reclaimed; adb/gradle polls honour the
    /// shared cancellation flag.
    ///
    /// # Errors
    /// [`Failure::Local`] at any stage; a journal record is cleared only
    /// after the row is provably removed, and cleanup failures compose
    /// into the returned error.
    pub fn start_session_until_journaled(
        &self,
        serial: &str,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        journal: Arc<dyn ForwardJournal>,
    ) -> Result<AndroidSession, Failure> {
        let adb = self.adb.with_cancellation(cancelled);
        check_device_state(&adb, serial)?;
        let apk = ensure_apk(self.apk_override.as_deref(), &self.driver_dir, &adb)?;
        install(&adb, serial, &apk)?;
        enable_service(&adb, serial)?;
        let session = provision(&adb, serial)?;
        let local_port = forward::reserve_local_port()?;
        journal.record(serial, local_port, session.device_port)?;
        let forward_port = match create_forward(&adb, serial, local_port, session.device_port) {
            Ok(p) => p,
            Err(e) => {
                return Err(self.rollback_journal(
                    serial,
                    local_port,
                    session.device_port,
                    &journal,
                    e,
                ));
            }
        };
        self.finish_session(&adb, serial, apk, session, forward_port, journal)
    }

    /// Remove the exact forward row on an uncancelled adapter, then clear
    /// the journal — only a confirmed removal clears; failures compose
    /// into the returned error while the pending record survives.
    fn rollback_journal(
        &self,
        serial: &str,
        local_port: u16,
        device_port: u16,
        journal: &Arc<dyn ForwardJournal>,
        cause: Failure,
    ) -> Failure {
        let cleanup = self.adb.without_cancellation();
        if let Err(e) = remove_owned_forward(&cleanup, serial, local_port, device_port) {
            return Failure::local(
                format!(
                    "{}; forward cleanup failed: {}",
                    cause.message(),
                    e.message()
                ),
                e.render(),
            );
        }
        if let Err(e) = journal.clear(serial, local_port, device_port) {
            return Failure::local(
                format!(
                    "{}; forward journal clear failed: {}",
                    cause.message(),
                    e.message()
                ),
                e.render(),
            );
        }
        cause
    }

    /// Probe + bridge after the forward exists: cancellation is read
    /// from the working `work_adb` at each boundary, and failures remove
    /// only the exact `serial/local/remote` row on an uncancelled
    /// adapter, folding cleanup failure into the error.
    fn finish_session(
        &self,
        work_adb: &Adb,
        serial: &str,
        apk: PathBuf,
        session_meta: Provisioned,
        forward_port: u16,
        journal: Arc<dyn ForwardJournal>,
    ) -> Result<AndroidSession, Failure> {
        let result = (|| {
            if work_adb.is_cancelled() {
                return Err(Failure::local("operation interrupted", "rerun the command"));
            }
            probe_status(
                &format!("http://127.0.0.1:{forward_port}"),
                &session_meta.token,
            )?;
            if work_adb.is_cancelled() {
                return Err(Failure::local("operation interrupted", "rerun the command"));
            }
            let lifecycle = Arc::new(AdbLifecycle::new(self.adb.clone(), serial));
            let mut bridge = start_bridge(forward_port, &session_meta.token, lifecycle)?;
            if work_adb.is_cancelled() {
                bridge.stop();
                return Err(Failure::local("operation interrupted", "rerun the command"));
            }
            let url = format!("http://127.0.0.1:{}", bridge.port());
            Ok((bridge, url))
        })();
        match result {
            Ok((bridge, url)) => Ok(AndroidSession {
                adb: self.adb.clone(),
                serial: serial.to_owned(),
                url,
                token: session_meta.token,
                local_port: bridge.port(),
                forward_port,
                device_port: session_meta.device_port,
                apk_source: apk,
                bridge: Some(bridge),
                closed: false,
                journal,
                journal_pending: true,
            }),
            Err(e) => Err(self.rollback_journal(
                serial,
                forward_port,
                session_meta.device_port,
                &journal,
                e,
            )),
        }
    }
}

#[cfg(test)]
mod tests;
