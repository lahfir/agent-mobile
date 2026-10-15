//! Request parsing, constant-time auth, and envelope shaping.

use agent_mobile_core::error::Failure;

use super::{
    AUTH_MESSAGE, HEAD_CAP, Request, bearer_matches, error_envelope, read_request, success_envelope,
};

fn req_bytes(head: &str, body: &[u8]) -> Vec<u8> {
    let mut out = head.as_bytes().to_vec();
    out.extend_from_slice(body);
    out
}

#[test]
fn parses_post_preserving_raw_bytes() -> Result<(), Failure> {
    let raw = req_bytes(
        "POST /snapshot HTTP/1.1\r\nHost: x\r\nContent-Length: 7\r\n\r\n",
        b"{\"a\":1}",
    );
    let mut pair = std::io::Cursor::new(raw.clone());
    let req = read_request(&mut pair)?;
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/snapshot");
    assert_eq!(req.header("content-length").as_deref(), Some("7"));
    assert_eq!(req.raw, raw);
    Ok(())
}

#[test]
fn oversized_head_is_an_error() {
    let raw = vec![b'a'; 1 << 21];
    let mut pair = std::io::Cursor::new(raw);
    assert!(read_request(&mut pair).is_err());
}

#[test]
fn bearer_match_is_exact_and_constant_shaped() {
    assert!(bearer_matches("Bearer tok123", "tok123"));
    assert!(!bearer_matches("Bearer tok1234", "tok123"));
    assert!(!bearer_matches("bearer tok123", "tok123"));
    assert!(!bearer_matches("Bearer other", "tok123"));
    assert!(!bearer_matches("", "tok123"));
}

#[test]
fn error_envelope_omits_command_for_auth() {
    let body = error_envelope(None, None, "UNAUTHORIZED", AUTH_MESSAGE);
    assert!(!body.contains("\"command\""));
    assert!(!body.contains("elapsed_ms"));
    assert!(body.contains("UNAUTHORIZED"));
    let with_cmd = error_envelope(Some("launch"), Some(9), "BAD_REQUEST", "nope");
    assert!(with_cmd.contains("\"command\":\"launch\""));
    assert!(with_cmd.contains("\"elapsed_ms\":9"));
}

#[test]
fn success_envelope_carries_data() {
    let body = success_envelope(
        "terminate",
        3,
        agent_mobile_core::contract::Data::Terminate(agent_mobile_core::contract::Terminate {
            terminated: "a.b".to_owned(),
        }),
    );
    assert!(body.contains("\"ok\":true"));
    assert!(body.contains("\"data\":{\"terminated\":\"a.b\"}"));
}

#[test]
fn header_lookup_is_case_insensitive() -> Result<(), Failure> {
    let raw = req_bytes(
        "POST /x HTTP/1.1\r\nX-Agent-Mobile-Version: 1\r\nContent-Length: 0\r\n\r\n",
        b"",
    );
    let mut pair = std::io::Cursor::new(raw);
    let req = read_request(&mut pair)?;
    assert_eq!(req.header("x-agent-mobile-version").as_deref(), Some("1"));
    assert_eq!(req.content_length(), Some(0));
    Ok(())
}

#[test]
fn body_larger_than_cap_is_an_error() {
    let head = "POST /x HTTP/1.1\r\nContent-Length: 20000000\r\n\r\n";
    let raw = req_bytes(head, &[]);
    let mut pair = std::io::Cursor::new(raw);
    assert!(read_request(&mut pair).is_err());
}

#[test]
fn reads_exact_body_no_more() -> Result<(), Failure> {
    let raw = req_bytes("POST /x HTTP/1.1\r\nContent-Length: 3\r\n\r\n", b"abcdefgh");
    let mut pair = std::io::Cursor::new(raw);
    let req = read_request(&mut pair)?;
    assert_eq!(req.body, b"abc");
    assert!(req.raw.ends_with(b"abc"));
    Ok(())
}

fn parse(text: &str) -> std::io::Result<Request> {
    read_request(&mut std::io::Cursor::new(text.as_bytes().to_vec()))
}

#[test]
fn request_line_needs_exact_three_parts_and_http1() {
    for bad in [
        "POST /x\r\n\r\n",
        "POST /x HTTP/2\r\n\r\n",
        "POST /x HTTP/1.1 extra\r\n\r\n",
        "POST\r\n\r\n",
    ] {
        assert!(parse(bad).is_err(), "{bad:?} accepted");
    }
    assert!(parse("GET /x HTTP/1.0\r\n\r\n").is_ok());
    assert!(parse("POST /x HTTP/1.1\r\n\r\n").is_ok());
}

#[test]
fn malformed_headers_are_rejected() {
    for bad in [
        "POST /x HTTP/1.1\r\nNoColonHere\r\n\r\n",
        "POST /x HTTP/1.1\r\n: emptyname\r\n\r\n",
        "POST /x HTTP/1.1\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n",
        "POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nA",
        "POST /x HTTP/1.1\r\nContent-Length: nope\r\n\r\n",
    ] {
        assert!(parse(bad).is_err(), "{bad:?} accepted");
    }
}

#[test]
fn head_cap_counts_the_terminator() {
    let mut raw = Vec::with_capacity(HEAD_CAP + 8);
    raw.extend_from_slice(b"POST /x HTTP/1.1\r\nx: ");
    let pad = HEAD_CAP - raw.len() - 2;
    raw.extend(std::iter::repeat_n(b'a', pad));
    raw.extend_from_slice(b"\r\n\r\n");
    assert!(read_request(&mut std::io::Cursor::new(raw)).is_err());
}

#[test]
fn request_reads_when_terminator_splits_across_reads() -> std::io::Result<()> {
    use std::io::Read as _;
    let head_a: &[u8] = b"POST /status HTTP/1.1\r\nContent-Length: 4\r\n\r";
    let head_b: &[u8] = b"\nBODYtrailing-ignored";
    let mut reader = head_a.chain(head_b);
    let req = read_request(&mut reader)?;
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/status");
    assert_eq!(req.body, b"BODY");
    assert_eq!(
        req.raw,
        b"POST /status HTTP/1.1\r\nContent-Length: 4\r\n\r\nBODY"
    );
    Ok(())
}
