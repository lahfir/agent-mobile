//! Text rendering (KTD12, R5, R10): the snapshot header plus printed-node
//! lines identical to the driver's own text mode, and one-line shapes for the
//! non-tree replies. Callers gate on `env.ok` — error envelopes render
//! through [`crate::error::Failure::render`], not here.

use crate::contract::{Data, Envelope, Node, Snapshot};

/// Render one successful reply for text mode; `screenshot` yields raw base64
/// for stdout.
#[must_use]
pub fn render(env: &Envelope) -> String {
    match &env.data {
        Some(Data::Snapshot(snap)) => render_snapshot(env, snap),
        Some(Data::Status(s)) => {
            let snap = if s.snapshot_id.is_empty() {
                "-".to_owned()
            } else {
                format!("@{}", s.snapshot_id)
            };
            format!(
                "app={} device=\"{}\" os={} snapshot={} elapsed_ms={}",
                s.app,
                s.device,
                s.os,
                snap,
                elapsed(env)
            )
        }
        Some(Data::Terminate(t)) => {
            format!(
                "terminated={} app=com.apple.springboard elapsed_ms={}",
                t.terminated,
                elapsed(env)
            )
        }
        Some(Data::Screenshot(s)) => s.png_base64.clone(),
        None => "ok".to_owned(),
    }
}

/// The confirmation line for `screenshot <path>` (KTD12): byte count plus the
/// path written.
#[must_use]
pub fn screenshot_written(path: &str, bytes: usize) -> String {
    format!("screenshot={path} bytes={bytes}")
}

fn elapsed(env: &Envelope) -> String {
    env.elapsed_ms
        .map_or_else(|| "-".to_owned(), |v| v.to_string())
}

fn render_snapshot(env: &Envelope, snap: &Snapshot) -> String {
    let mut out = format!(
        "app={} snapshot=@{} refs={} settled={} reads={} elapsed_ms={}",
        snap.app,
        snap.snapshot_id,
        snap.ref_count,
        snap.settled,
        snap.reads,
        elapsed(env)
    );
    if !snap.complete {
        out.push_str(" complete=false");
    }
    out.push('\n');
    walk(&snap.tree, 0, &mut out);
    out
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the driver truncates bounds with Int(); screen sizes fit i64"
)]
fn trunc(v: f64) -> i64 {
    v as i64
}

fn walk(node: &Node, printed_depth: usize, out: &mut String) {
    let printed =
        !node.name.is_empty() || !node.value.is_empty() || !node.available_actions.is_empty();
    if printed {
        let indent = "  ".repeat(printed_depth);
        let value = if node.value.is_empty() {
            String::new()
        } else {
            format!(" value=\"{}\"", node.value)
        };
        let states = if node.states.is_empty() {
            String::new()
        } else {
            format!(" [{}]", node.states.join(","))
        };
        let line = format!(
            "{indent}{} {} \"{}\"{} at={},{} size={}x{}{}\n",
            node.ref_id,
            node.role,
            node.name,
            value,
            trunc(node.bounds.x),
            trunc(node.bounds.y),
            trunc(node.bounds.width),
            trunc(node.bounds.height),
            states
        );
        out.push_str(&line);
    }
    let next = printed_depth + usize::from(printed);
    for child in &node.children {
        walk(child, next, out);
    }
}
