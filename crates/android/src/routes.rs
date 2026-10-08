//! Local `launch`/`terminate` routes for the bridge: shared auth,
//! version, and JSON-body gating, then the `adb` action plus the upstream
//! `snapshot`/`status` call that completes the wire-visible reply.

use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use agent_mobile_core::contract::{Data, Envelope, PROTOCOL_VERSION, Terminate};
use agent_mobile_core::error::ErrorCode;
use agent_mobile_core::wire::Wire;
use serde_json::{Map, json};

use crate::driver::{SecretToken, is_status_envelope};
use crate::lifecycle::{LifecycleControl, LifecycleError, valid_package};

/// Upstream address for the forwarded device listener.
fn upstream(upstream_port: u16) -> String {
    format!("http://127.0.0.1:{upstream_port}")
}

/// HTTP status for a wire error code: auth stays 401, registered
/// post-dispatch errors stay 409, and unknown codes become 500.
fn status_for_code(code: &str) -> &'static str {
    match ErrorCode::from_code(code) {
        Some(ErrorCode::Unauthorized) => "401 Unauthorized",
        Some(
            ErrorCode::BadRequest
            | ErrorCode::StaleRef
            | ErrorCode::AmbiguousTarget
            | ErrorCode::UnknownCommand,
        ) => "409 Conflict",
        _ => "500 Internal Server Error",
    }
}

use crate::proxy::{
    AUTH_MESSAGE, Request, UPSTREAM_TIMEOUT, bearer_matches, elapsed_ms, error_envelope,
    http_response, http_response_typed, success_envelope,
};

/// Shared envelope write for the local routes.
fn respond(sock: &mut TcpStream, status: &str, body: &str) {
    let _ = sock.write_all(&http_response(status, body));
}

/// Auth/version/body gate shared by `launch` and `terminate`; returns the
/// parsed JSON object when the request may proceed.
fn gate(
    sock: &mut TcpStream,
    req: &Request,
    verb: &str,
    token: &SecretToken,
    started: Instant,
) -> Option<serde_json::Value> {
    let authed = req
        .header("authorization")
        .is_some_and(|h| bearer_matches(&h, token.as_str()));
    if !authed {
        respond(
            sock,
            "401 Unauthorized",
            &error_envelope(None, None, "UNAUTHORIZED", AUTH_MESSAGE),
        );
        return None;
    }
    if req.method != "POST" {
        respond(
            sock,
            "405 Method Not Allowed",
            &error_envelope(
                Some(verb),
                Some(elapsed_ms(started)),
                "BAD_REQUEST",
                "verbs are POST only",
            ),
        );
        return None;
    }
    if req.header("x-agent-mobile-version").as_deref() != Some(PROTOCOL_VERSION) {
        respond(
            sock,
            "409 Conflict",
            &error_envelope(
                Some(verb),
                Some(elapsed_ms(started)),
                "BAD_REQUEST",
                "X-Agent-Mobile-Version must be 1",
            ),
        );
        return None;
    }
    if req.body.is_empty() {
        return Some(serde_json::Value::Object(Map::new()));
    }
    let body: serde_json::Value = match serde_json::from_slice::<serde_json::Value>(&req.body) {
        Ok(v) if v.is_object() => v,
        _ => {
            respond(
                sock,
                "409 Conflict",
                &error_envelope(
                    Some(verb),
                    Some(elapsed_ms(started)),
                    "BAD_REQUEST",
                    "body must be a JSON object",
                ),
            );
            return None;
        }
    };
    Some(body)
}

/// Local `launch`/`terminate`: enforce the shared gate, run the `adb`
/// action, then complete the wire-visible half upstream.
pub(crate) fn lifecycle_route(
    sock: &mut TcpStream,
    req: &Request,
    verb: &str,
    upstream_port: u16,
    token: &SecretToken,
    lifecycle: &Arc<dyn LifecycleControl>,
    started: Instant,
) {
    let Some(body) = gate(sock, req, verb, token, started) else {
        return;
    };
    let wants_text = req.header("accept").as_deref() == Some("text/plain");
    let wire = Wire::with_timeout(&upstream(upstream_port), token.as_str(), UPSTREAM_TIMEOUT);
    if verb == "launch" {
        launch_route(sock, &body, &wire, lifecycle, started, wants_text);
    } else {
        terminate_route(sock, &wire, lifecycle, started);
    }
}

/// Map a lifecycle failure to status + envelope.
fn lifecycle_fail(sock: &mut TcpStream, verb: &str, started: Instant, err: LifecycleError) {
    match err {
        LifecycleError::BadRequest(m) => respond(
            sock,
            "409 Conflict",
            &error_envelope(Some(verb), Some(elapsed_ms(started)), "BAD_REQUEST", &m),
        ),
        LifecycleError::Driver(m) => respond(
            sock,
            "500 Internal Server Error",
            &error_envelope(Some(verb), Some(elapsed_ms(started)), "DRIVER_ERROR", &m),
        ),
    }
}

/// Forward an upstream error envelope downstream, preserving its code
/// and message; `fallback_msg` covers envelopes without an error body.
fn forward_upstream_error(
    sock: &mut TcpStream,
    verb: &str,
    started: Instant,
    env: &Envelope,
    fallback_msg: &str,
) {
    let (code, msg) = env.error.as_ref().map_or_else(
        || ("DRIVER_ERROR".to_owned(), fallback_msg.to_owned()),
        |e| (e.code.clone(), e.message.clone()),
    );
    respond(
        sock,
        status_for_code(&code),
        &error_envelope(Some(verb), Some(elapsed_ms(started)), &code, &msg),
    );
}

/// Authenticated upstream `status` before any ADB side effect.
fn preflight_status(sock: &mut TcpStream, wire: &Wire, started: Instant) -> bool {
    match wire.call("status", &json!({})) {
        Ok(env) if is_status_envelope(&env) => true,
        Ok(env) if !env.ok => {
            forward_upstream_error(sock, "launch", started, &env, "status failed");
            false
        }
        _ => {
            respond(
                sock,
                "500 Internal Server Error",
                &error_envelope(
                    Some("launch"),
                    Some(elapsed_ms(started)),
                    "DRIVER_ERROR",
                    "pre-launch status probe failed",
                ),
            );
            false
        }
    }
}

/// `launch`: `bundle_id` → adb start → upstream `snapshot {app}` →
/// command/elapsed rewritten, snapshot data preserved.
fn launch_route(
    sock: &mut TcpStream,
    body: &serde_json::Value,
    wire: &Wire,
    lifecycle: &Arc<dyn LifecycleControl>,
    started: Instant,
    wants_text: bool,
) {
    let bundle = body.get("bundle_id").and_then(|v| v.as_str()).unwrap_or("");
    if bundle.is_empty() || !valid_package(bundle) {
        respond(
            sock,
            "409 Conflict",
            &error_envelope(
                Some("launch"),
                Some(elapsed_ms(started)),
                "BAD_REQUEST",
                "bundle_id must be a valid package name",
            ),
        );
        return;
    }
    if !preflight_status(sock, wire, started) {
        return;
    }
    if let Err(e) = lifecycle.launch(bundle) {
        lifecycle_fail(sock, "launch", started, e);
        return;
    }
    match wire.call("snapshot", &json!({"app": bundle})) {
        Err(_) => respond(
            sock,
            "500 Internal Server Error",
            &error_envelope(
                Some("launch"),
                Some(elapsed_ms(started)),
                "DRIVER_ERROR",
                "post-launch snapshot failed",
            ),
        ),
        Ok(env) if !env.ok => {
            forward_upstream_error(sock, "launch", started, &env, "snapshot failed");
        }
        Ok(env) => match env.data {
            Some(Data::Snapshot(snap)) if snap.app == bundle => {
                let elapsed = elapsed_ms(started);
                if wants_text {
                    let body = format!(
                        "app={} snapshot=@{} refs={} settled={} reads={} elapsed_ms={}{}\n{}\n",
                        snap.app,
                        snap.snapshot_id,
                        snap.ref_count,
                        snap.settled,
                        snap.reads,
                        elapsed,
                        if snap.complete { "" } else { " complete=false" },
                        snap.text
                    );
                    let _ = sock.write_all(&http_response_typed("200 OK", &body, "text/plain"));
                } else {
                    respond(
                        sock,
                        "200 OK",
                        &success_envelope("launch", elapsed, Data::Snapshot(snap)),
                    );
                }
            }
            _ => respond(
                sock,
                "500 Internal Server Error",
                &error_envelope(
                    Some("launch"),
                    Some(elapsed_ms(started)),
                    "DRIVER_ERROR",
                    "app did not come to the foreground",
                ),
            ),
        },
    }
}

/// `terminate`: upstream `status` → the observed app only → adb stop →
/// `Data::Terminate`.
fn terminate_route(
    sock: &mut TcpStream,
    wire: &Wire,
    lifecycle: &Arc<dyn LifecycleControl>,
    started: Instant,
) {
    let app = match wire.call("status", &json!({})) {
        Err(_) => {
            respond(
                sock,
                "500 Internal Server Error",
                &error_envelope(
                    Some("terminate"),
                    Some(elapsed_ms(started)),
                    "DRIVER_ERROR",
                    "status probe failed before terminate",
                ),
            );
            return;
        }
        Ok(env) if !env.ok => {
            forward_upstream_error(sock, "terminate", started, &env, "status failed");
            return;
        }
        Ok(env) => match env.data {
            Some(Data::Status(st)) if !st.app.is_empty() => st.app,
            _ => {
                respond(
                    sock,
                    "500 Internal Server Error",
                    &error_envelope(
                        Some("terminate"),
                        Some(elapsed_ms(started)),
                        "DRIVER_ERROR",
                        "no foreground app to terminate",
                    ),
                );
                return;
            }
        },
    };
    if let Err(e) = lifecycle.terminate(&app) {
        lifecycle_fail(sock, "terminate", started, e);
        return;
    }
    respond(
        sock,
        "200 OK",
        &success_envelope(
            "terminate",
            elapsed_ms(started),
            Data::Terminate(Terminate { terminated: app }),
        ),
    );
}

#[cfg(test)]
mod body_tests;
#[cfg(test)]
mod spec_tests;
#[cfg(test)]
mod tests;
