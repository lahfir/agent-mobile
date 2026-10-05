//! Byte-level HTTP plumbing for the bridge: request reads preserve raw
//! bytes so non-lifecycle verbs relay untouched, auth comparison is
//! constant-time, and locally minted replies carry the protocol-v1
//! envelope shape the on-device service already speaks.

use std::io::{self, Read};

use agent_mobile_core::contract::PROTOCOL_VERSION;
use serde_json::{Value, json};

/// Request head cap: 1 MiB.
pub(crate) const HEAD_CAP: usize = 1 << 20;
/// Request body cap: 16 MiB.
pub(crate) const BODY_CAP: usize = 16 << 20;
/// Upstream reply cap: 64 MiB.
pub(crate) const REPLY_CAP: usize = 64 << 20;
/// Per-socket IO deadline.
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Same words the on-device service emits for auth failures.
pub(crate) const AUTH_MESSAGE: &str = "Authorization: Bearer <token> required";

/// One fully-read request with its raw bytes preserved for verbatim relay.
pub(crate) struct Request {
    /// HTTP method, e.g. `POST`.
    pub(crate) method: String,
    /// Request target, e.g. `/snapshot`.
    pub(crate) path: String,
    /// Original head bytes + exact body — what gets relayed upstream.
    pub(crate) raw: Vec<u8>,
    /// Decoded body bytes (content-length bounded).
    pub(crate) body: Vec<u8>,
    headers: Vec<(String, String)>,
}

impl Request {
    /// First header value, case-insensitive name.
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let wanted = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| n == &wanted)
            .map(|(_, v)| v.clone())
    }

    /// Parsed `Content-Length` when present and numeric.
    #[cfg(test)]
    pub(crate) fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
    }
}

/// Read one request: head up to [`HEAD_CAP`], then exactly
/// `Content-Length` body bytes up to [`BODY_CAP`]. Raw bytes are preserved
/// for relay; anything malformed is a plain io error — callers map it to
/// `BAD_REQUEST` or drop the client.
///
/// # Errors
/// io failures, oversized heads/bodies, or malformed request lines.
pub(crate) fn read_request(reader: &mut impl Read) -> io::Result<Request> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    let head_end = read_head(reader, &mut raw, &mut buf)?;
    let head_text = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let (method, path, headers) = parse_head(&head_text)?;
    let want = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if want > BODY_CAP {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    while raw.len() < head_end + 4 + want {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "body truncated",
            ));
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > head_end + 4 + BODY_CAP {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "body overflow"));
        }
    }
    raw.truncate(head_end + 4 + want);
    let body = raw[head_end + 4..].to_vec();
    Ok(Request {
        method,
        path,
        raw,
        body,
        headers,
    })
}

/// Offset of the `\r\n\r\n` terminator, scanning only bytes at or after
/// `scan_from` — shared by the request and reply readers so the window
/// search never re-covers already-scanned bytes.
#[must_use]
pub(crate) fn find_head_end(raw: &[u8], scan_from: usize) -> Option<usize> {
    raw.get(scan_from..)?
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| scan_from + position)
}

/// Read until the `\r\n\r\n` terminator or [`HEAD_CAP`]; returns the head
/// end offset. `scan_from` resumes the search 3 bytes back from the last
/// tip — a split terminator's first half could sit right at the boundary.
fn read_head(reader: &mut impl Read, raw: &mut Vec<u8>, buf: &mut [u8]) -> io::Result<usize> {
    let mut scan_from = 0;
    loop {
        if let Some(pos) = find_head_end(raw, scan_from) {
            if pos + 4 > HEAD_CAP {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
            }
            return Ok(pos);
        }
        if raw.len() >= HEAD_CAP {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
        }
        scan_from = raw.len().saturating_sub(3);
        let n = reader.read(buf)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        raw.extend_from_slice(&buf[..n]);
    }
}

/// Method, path, and header pairs decoded from the head.
type ParsedHead = (String, String, Vec<(String, String)>);

/// Parse request line + headers from head text: exactly
/// `METHOD PATH HTTP/1.0|1.1`, every header line `name: value`, exactly
/// one numeric `Content-Length`, and no `Transfer-Encoding`.
fn parse_head(head: &str) -> io::Result<ParsedHead> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "bad request");
    let mut words = request_line.split_whitespace();
    let (Some(method), Some(path), Some(version)) = (words.next(), words.next(), words.next())
    else {
        return Err(bad());
    };
    if words.next().is_some() || (version != "HTTP/1.0" && version != "HTTP/1.1") {
        return Err(bad());
    }
    let mut headers = Vec::new();
    let mut content_lengths = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(bad());
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err(bad());
        }
        if name == "content-length" {
            content_lengths += 1;
            if content_lengths > 1 || value.trim().parse::<usize>().is_err() {
                return Err(bad());
            }
        }
        if name == "transfer-encoding" {
            return Err(bad());
        }
        headers.push((name, value.trim().to_owned()));
    }
    Ok((method.to_owned(), path.to_owned(), headers))
}

/// Constant-time bearer check: compares the full header against
/// `Bearer <token>` without early exit, matching the service's
/// `MessageDigest.isEqual` behavior.
pub(crate) fn bearer_matches(header: &str, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    let a = header.as_bytes();
    let b = expected.as_bytes();
    let mut diff = a.len() ^ b.len();
    let max = a.len().max(b.len());
    for i in 0..max {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// Protocol-v1 failure envelope; `command`/`elapsed_ms` are omitted for
/// pre-dispatch 401s, matching the service's auth-failure shape.
#[must_use]
pub(crate) fn error_envelope(
    command: Option<&str>,
    elapsed_ms: Option<u64>,
    code: &str,
    message: &str,
) -> String {
    let mut env = json!({"version": PROTOCOL_VERSION, "ok": false});
    if let Some(c) = command {
        env["command"] = json!(c);
    }
    if let Some(ms) = elapsed_ms {
        env["elapsed_ms"] = json!(ms);
    }
    env["error"] = json!({"code": code, "message": message});
    env.to_string()
}

/// Protocol-v1 success envelope carrying `data`; callers must serialize
/// to [`Value`] themselves so a failure can never become `ok` + `null`.
#[must_use]
pub(crate) fn success_envelope(command: &str, elapsed_ms: u64, data: &Value) -> String {
    json!({
        "version": PROTOCOL_VERSION,
        "ok": true,
        "command": command,
        "elapsed_ms": elapsed_ms,
        "data": data,
    })
    .to_string()
}

/// Whole-request elapsed in milliseconds.
#[must_use]
pub(crate) fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A complete HTTP/1.1 reply buffer.
#[must_use]
pub(crate) fn http_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests;
