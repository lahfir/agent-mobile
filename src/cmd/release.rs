//! Stop a recorded driver session without stopping its device.

use std::time::{Duration, Instant};

use super::{Call, Reply};
use agent_mobile_core::error::Failure;
use agent_mobile_core::state::StateStore;

const RELEASE_BUDGET: Duration = Duration::from_secs(30);

/// Release the selected live driver, or the only live driver when omitted.
///
/// # Errors
/// State access failures or an ambiguous selection.
pub fn exec(call: &Call) -> Result<Reply, Failure> {
    let store = StateStore::new()?;
    let state = store.load();
    let selector = call.str("device");
    let selected = selector
        .map(|raw| state.resolve_live_device_key(raw))
        .transpose()?
        .flatten();
    if selector.is_some() && selected.is_none() {
        if let Some(raw) = selector
            && state.devices.get(raw).is_some_and(|entry| {
                !agent_mobile_core::process::process_matches(
                    entry.pid,
                    entry.process_started_at.as_deref(),
                )
            })
        {
            return Ok(Reply::Text(format!("stale driver row for {raw}")));
        }
        return Ok(Reply::Text("no driver running".to_owned()));
    }
    let live: Vec<_> = state
        .devices
        .iter()
        .filter(|(key, entry)| {
            selected
                .as_deref()
                .is_none_or(|device| device == key.as_str())
                && agent_mobile_core::process::process_matches(
                    entry.pid,
                    entry.process_started_at.as_deref(),
                )
        })
        .collect();
    let Some((key, entry)) = live.first().copied() else {
        return Ok(Reply::Text("no driver running".to_owned()));
    };
    if selected.is_none() && live.len() > 1 {
        return Err(Failure::local(
            "more than one driver is running",
            "select one with `agent-mobile release --device <key>`",
        ));
    }
    let _ = agent_mobile_core::process::terminate(entry.pid);
    let deadline = Instant::now() + RELEASE_BUDGET;
    loop {
        if store.entry(key).is_none() {
            return Ok(Reply::Text(format!("released {key}")));
        }
        if Instant::now() >= deadline {
            return Ok(Reply::Text(
                "release in progress; the driver is still shutting down".to_owned(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
