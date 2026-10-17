//! Bound and frame untrusted driver text before it enters a tool result.

use serde_json::{Value, json};

const LIMIT: usize = 32_768;
const LINE_LIMIT: usize = 2_048;
const HEADER: &str = "<untrusted-data>\n";
const FOOTER: &str = "\n</untrusted-data>";
const TRUNCATED: &str = "\n[TRUNCATED: use snapshot max_depth to reduce the reply]";

/// One text item, with a byte cap including its framing and truncation notice.
pub fn text(raw: &str, error: bool) -> Value {
    let mut out = String::from(HEADER);
    let mut line_bytes = 0;
    let mut truncated = false;
    for c in raw.chars() {
        if c.is_control() && c != '\n'
            || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            continue;
        }
        let mut bytes = [0; 4];
        let escaped = match c {
            '<' => "&lt;",
            '>' => "&gt;",
            _ => c.encode_utf8(&mut bytes),
        };
        if out.len() + escaped.len() + FOOTER.len() + TRUNCATED.len() > LIMIT {
            truncated = true;
            break;
        }
        if c == '\n' {
            line_bytes = 0;
        } else if line_bytes + escaped.len() > LINE_LIMIT {
            truncated = true;
            continue;
        } else {
            line_bytes += escaped.len();
        }
        out.push_str(escaped);
    }
    if truncated {
        out.push_str(TRUNCATED);
    }
    out.push_str(FOOTER);
    json!({"content":[{"type":"text", "text":out}], "isError":error})
}

/// Remove configured credentials even when a remote failure echoes them.
pub fn redact(mut text: String) -> String {
    for key in ["AGENT_MOBILE_URL", "AGENT_MOBILE_TOKEN"] {
        if let Ok(secret) = std::env::var(key)
            && !secret.is_empty()
        {
            text = text.replace(&secret, "[redacted]");
        }
    }
    let Ok(store) = agent_mobile_core::state::StateStore::new() else {
        return text;
    };
    for entry in store.load().devices.values() {
        if !entry.url.is_empty() {
            text = text.replace(&entry.url, "[redacted]");
        }
        if agent_mobile_core::secret::validate_token_file_name(&entry.token_file).is_ok()
            && let Ok(secret) = std::fs::read_to_string(store.token_path(&entry.token_file))
            && !secret.trim().is_empty()
        {
            text = text.replace(secret.trim(), "[redacted]");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_mobile_core::error::Failure;

    #[test]
    fn replies_bound_long_nodes_and_total_utf8_bytes() -> Result<(), Failure> {
        for raw in ["界".repeat(LIMIT), "node\n".repeat(LIMIT)] {
            let result = text(&raw, false);
            let output = result["content"][0]["text"]
                .as_str()
                .ok_or_else(|| Failure::local("missing text", "fix test"))?;
            assert!(output.len() <= LIMIT);
            assert!(output.contains(TRUNCATED));
            assert!(output.starts_with(HEADER) && output.ends_with(FOOTER));
            assert!(output.lines().all(|line| line.len() <= LINE_LIMIT));
        }
        Ok(())
    }
}
