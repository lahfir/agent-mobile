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
        if c != '\n' && unsafe_char(c) {
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

/// Clean every string leaf of structured output like text content (R13):
/// credentials redacted, control and bidi characters dropped, each string
/// capped at a line's bound.
pub fn clean(value: Value) -> Value {
    walk(value, &secrets())
}

fn walk(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(s) => {
            let s = scrub(s, secrets);
            let mut out = String::new();
            for c in s.chars().filter(|c| !unsafe_char(*c)) {
                if out.len() + c.len_utf8() > LINE_LIMIT {
                    break;
                }
                out.push(c);
            }
            Value::String(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(|v| walk(v, secrets)).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, walk(v, secrets)))
                .collect(),
        ),
        other => other,
    }
}

fn unsafe_char(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Remove configured credentials even when a remote failure echoes them.
pub fn redact(text: String) -> String {
    scrub(text, &secrets())
}

fn scrub(mut text: String, secrets: &[String]) -> String {
    for secret in secrets {
        text = text.replace(secret.as_str(), "[redacted]");
    }
    text
}

/// Driver URLs and tokens from the environment and the session store.
fn secrets() -> Vec<String> {
    let mut out: Vec<String> = ["AGENT_MOBILE_URL", "AGENT_MOBILE_TOKEN"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .collect();
    if let Ok(store) = agent_mobile_core::state::StateStore::new() {
        for entry in store.load().devices.values() {
            out.push(entry.url.clone());
            if agent_mobile_core::secret::validate_token_file_name(&entry.token_file).is_ok()
                && let Ok(secret) = std::fs::read_to_string(store.token_path(&entry.token_file))
            {
                out.push(secret.trim().to_owned());
            }
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_mobile_core::error::Failure;

    #[test]
    fn structured_strings_are_redacted_and_cleaned() {
        let secrets = ["http://127.0.0.1:9".to_owned()];
        let value = json!({"devices": [{"serving": "http://127.0.0.1:9", "name": "a\u{1b}b"}]});
        let out = walk(value, &secrets);
        assert_eq!(out["devices"][0]["serving"], "[redacted]");
        assert_eq!(out["devices"][0]["name"], "ab");
    }

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
