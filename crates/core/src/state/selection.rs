//! Live-row device alias resolution: a selector maps to a state row's key
//! with no platform discovery and no token-file reads.

use crate::error::Failure;

use super::{SessionEntry, State, StateStore};

impl State {
    /// The stored default selectors in canonical order — key then display
    /// name, distinct — without cloning. An explicit `--device` is the
    /// sole selector and handled by callers, not this iterator.
    pub fn default_device_selectors(&self) -> impl Iterator<Item = &str> {
        let key = self.default_device_key.as_deref();
        [
            key,
            self.default_device
                .as_deref()
                .filter(move |display| Some(*display) != key),
        ]
        .into_iter()
        .flatten()
    }

    /// Resolve `query` to a live session row's key: exact row key first,
    /// then a unique stable alias (`device_id`, `serial`, or
    /// `platform:device_id`), then a unique `device_name`. Dead rows —
    /// including markerless legacy rows — never match.
    ///
    /// # Errors
    /// Returns [`Failure::Usage`] when a stable alias or name matches more
    /// than one live row.
    pub fn resolve_live_device_key(&self, query: &str) -> Result<Option<String>, Failure> {
        let live: Vec<(&String, &SessionEntry)> = self
            .devices
            .iter()
            .filter(|(_, e)| {
                crate::process::process_matches(e.pid, e.process_started_at.as_deref())
            })
            .collect();
        if live.iter().any(|(k, _)| k.as_str() == query) {
            return Ok(Some(query.to_owned()));
        }
        let stable = live.iter().filter(|(_, e)| {
            e.device_id.as_deref() == Some(query)
                || e.serial.as_deref() == Some(query)
                || e.device_id
                    .as_deref()
                    .zip(e.platform.as_deref())
                    .is_some_and(|(id, p)| format!("{p}:{id}") == query)
        });
        if let Some(k) = unique_alias(query, stable)? {
            return Ok(Some(k.clone()));
        }
        let named = live
            .iter()
            .filter(|(_, e)| e.device_name.as_deref() == Some(query));
        Ok(unique_alias(query, named)?.cloned())
    }
}

/// Return a unique alias match, or reject an ambiguous one.
fn unique_alias<'a>(
    query: &str,
    keys: impl Iterator<Item = &'a (&'a String, &'a SessionEntry)>,
) -> Result<Option<&'a String>, Failure> {
    let matches: Vec<&String> = keys.map(|(k, _)| *k).collect();
    if matches.len() > 1 {
        return Err(Failure::usage(format!(
            "device selector '{query}' matches {} live sessions ({}); use the collision-free state key",
            matches.len(),
            matches
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(matches.into_iter().next())
}

impl StateStore {
    /// Single-load alias resolution; see [`State::resolve_live_device_key`].
    ///
    /// # Errors
    /// Returns [`Failure::Usage`] when a stable alias or name matches more
    /// than one live row.
    pub fn resolve_live_device_key(&self, query: &str) -> Result<Option<String>, Failure> {
        self.load().resolve_live_device_key(query)
    }
}

/// The selector a request resolves against: explicit `device`, else the
/// stored defaults in `key` then `display` order. A complete env pair
/// short-circuits to the first selector as metadata only; otherwise each
/// selector walks the live alias map and the first live hit wins, falling
/// back to the first selector so lazy-start keeps its miss shape.
///
/// # Errors
/// Returns [`Failure::Usage`] when a live alias is ambiguous.
pub(super) fn choose_device_key(
    state: &State,
    device: Option<&str>,
    env_pair: bool,
) -> Result<Option<String>, Failure> {
    let selectors: Vec<&str> = match device {
        Some(d) => vec![d],
        None => state.default_device_selectors().collect(),
    };
    if env_pair {
        return Ok(selectors.first().map(|s| (*s).to_owned()));
    }
    for selector in &selectors {
        if let Some(key) = state.resolve_live_device_key(selector)? {
            return Ok(Some(key));
        }
    }
    Ok(selectors.first().map(|s| (*s).to_owned()))
}
