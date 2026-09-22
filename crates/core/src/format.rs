//! Text rendering (KTD12, R5, R10): the snapshot header plus printed-node
//! lines identical to the driver's own text mode, and one-line shapes for the
//! non-tree replies. Callers gate on `env.ok` — error envelopes render
//! through [`crate::error::Failure::render`], not here.

use std::borrow::Cow;

use crate::contract::{Data, Envelope, Node, Snapshot};

/// Render one successful reply for text mode; `screenshot` yields raw base64
/// for stdout. Borrows where the payload is already a `String` — a PNG is
/// megabytes, not worth cloning.
#[must_use]
pub fn render(env: &Envelope) -> Cow<'_, str> {
    match &env.data {
        Some(Data::Snapshot(snap)) => Cow::Owned(render_snapshot(env, snap)),
        Some(Data::Status(s)) => {
            let snap = if s.snapshot_id.is_empty() {
                "-".to_owned()
            } else {
                format!("@{}", s.snapshot_id)
            };
            Cow::Owned(format!(
                "app={} device=\"{}\" os={} snapshot={} elapsed_ms={}",
                clean(&s.app),
                clean(&s.device),
                clean(&s.os),
                snap,
                elapsed(env)
            ))
        }
        Some(Data::Terminate(t)) => Cow::Owned(format!(
            "terminated={} elapsed_ms={}",
            t.terminated,
            elapsed(env)
        )),
        Some(Data::Screenshot(s)) => Cow::Borrowed(s.png_base64.as_str()),
        None => Cow::Borrowed("ok"),
    }
}

/// The driver's text listing re-rendered from a (possibly trimmed) tree —
/// `walk`'s line format joined without a trailing newline, matching the
/// driver's `text` field so `--max-depth` can refresh `snap.text`.
#[must_use]
pub fn tree_lines(tree: &Node) -> String {
    let mut out = String::new();
    walk(tree, 0, &mut out);
    out.truncate(out.len().saturating_sub(1));
    out
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

/// Replace control characters in app-controlled strings so a hostile or
/// buggy label cannot inject escape sequences into the terminal.
fn clean(s: &str) -> Cow<'_, str> {
    if s.chars().all(|c| !c.is_control()) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| if c.is_control() { '\u{fffd}' } else { c })
            .collect(),
    )
}

fn render_snapshot(env: &Envelope, snap: &Snapshot) -> String {
    let mut out = format!(
        "app={} snapshot=@{} refs={} settled={} reads={} elapsed_ms={}",
        clean(&snap.app),
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
            format!(" value=\"{}\"", clean(&node.value))
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
            clean(&node.name),
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
