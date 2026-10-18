//! One validator per argument kind (KTD2), shared by both surfaces: the CLI
//! turns its words into JSON first, then both run [`check_args`].

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;
use serde_json::{Map, Value, json};

use super::{Arg, Args, Kind, Verb};

const TEXT_LIMIT: usize = 65_536;
const REF_FIX: &str = "use a ref from the most recent snapshot or action reply, like @snapshot:e1";

/// A field-named argument fault: what was wrong and how to fix it.
#[derive(Debug, PartialEq, Eq)]
pub struct Fault {
    /// Argument path, e.g. `target.y`.
    pub field: String,
    /// What was wrong.
    pub problem: String,
    /// The fix.
    pub fix: String,
}

impl Fault {
    fn new(field: &str, problem: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            field: field.to_owned(),
            problem: problem.into(),
            fix: fix.into(),
        }
    }

    /// The CLI form: a usage failure that exits 2.
    #[must_use]
    pub fn usage(&self) -> Failure {
        Failure::usage(format!("{}: {}; {}", self.field, self.problem, self.fix))
    }

    /// The MCP form: tool-result text naming the field and the fix (R10).
    #[must_use]
    pub fn tool_text(&self) -> String {
        format!(
            "BAD_REQUEST: {}: {}\nnext: {}",
            self.field, self.problem, self.fix
        )
    }
}

/// Every argument key `verb` accepts, globals included; a path is a CLI-only key.
#[must_use]
pub fn keys(verb: &Verb, cli: bool) -> Vec<&'static str> {
    let mut keys = Vec::new();
    for arg in verb.args {
        if arg.kind == Kind::Path && !cli {
            continue;
        }
        if arg.kind == Kind::RefThenText {
            keys.extend(["ref", "text"]);
        } else {
            keys.push(arg.name);
        }
    }
    keys.extend(verb.globals.iter().map(|g| g.arg().name));
    keys
}

/// Validate a full argument object for `verb` as `cli` or MCP input:
/// unknown keys, missing required arguments, every value, and defaults.
///
/// # Errors
/// The first [`Fault`], naming the argument.
pub fn check_args(verb: &Verb, input: &Map<String, Value>, cli: bool) -> Result<Args, Fault> {
    let allowed = keys(verb, cli);
    if let Some(key) = input.keys().find(|k| !allowed.contains(&k.as_str())) {
        let fix = if allowed.is_empty() {
            format!("{} takes no arguments", verb.name)
        } else {
            format!("use only: {}", allowed.join(", "))
        };
        return Err(Fault::new(key, "unknown argument", fix));
    }
    let mut out = Args::new();
    for arg in verb.args {
        if arg.kind == Kind::RefThenText {
            ref_then_text(input, &mut out)?;
            continue;
        }
        match (input.get(arg.name), arg.kind) {
            (Some(value), _) => {
                out.insert(arg.name.to_owned(), check(arg, value)?);
            }
            (
                None,
                Kind::Number {
                    default: Some(d), ..
                },
            ) => {
                out.insert(arg.name.to_owned(), json!(d));
            }
            (None, _) if arg.required => {
                return Err(Fault::new(arg.name, "missing required argument", arg.help));
            }
            (None, _) => {}
        }
    }
    for global in verb.globals {
        let arg = global.arg();
        if let Some(value) = input.get(arg.name) {
            out.insert(arg.name.to_owned(), check(&arg, value)?);
        }
    }
    Ok(out)
}

fn ref_then_text(input: &Map<String, Value>, out: &mut Args) -> Result<(), Fault> {
    let text = input
        .get("text")
        .ok_or_else(|| Fault::new("text", "missing required argument", "give the text to type"))?;
    let valid = text
        .as_str()
        .is_some_and(|s| s.chars().count() <= TEXT_LIMIT);
    if !valid {
        return Err(Fault::new(
            "text",
            "must be a string",
            format!("give at most {TEXT_LIMIT} characters"),
        ));
    }
    out.insert("text".to_owned(), text.clone());
    if let Some(reference) = input.get("ref") {
        out.insert("ref".to_owned(), parse_ref("ref", reference)?);
    }
    Ok(())
}

/// Validate one value for `arg` and return its normalized form.
///
/// # Errors
/// A [`Fault`] naming the argument and the fix.
pub fn check(arg: &Arg, value: &Value) -> Result<Value, Fault> {
    let name = arg.name;
    match arg.kind {
        Kind::Text(max) => match value.as_str() {
            Some(s) if !s.is_empty() && s.chars().count() <= max => Ok(value.clone()),
            _ => Err(Fault::new(
                name,
                "must be a non-empty string",
                format!("give 1 to {max} characters"),
            )),
        },
        Kind::Ref => parse_ref(name, value),
        Kind::Number {
            gt, max, except, ..
        } => number(name, value, gt, max, except),
        Kind::Integer => match value.as_u64().filter(|n| u32::try_from(*n).is_ok()) {
            Some(_) => Ok(value.clone()),
            None => Err(Fault::new(
                name,
                "must be a whole number",
                format!("give 0 to {}", u32::MAX),
            )),
        },
        Kind::Enum(allowed) => match value.as_str() {
            Some(s) if allowed.contains(&s) => Ok(value.clone()),
            _ => Err(Fault::new(
                name,
                "not an allowed value",
                format!("use one of: {}", allowed.join(", ")),
            )),
        },
        Kind::AppId => match value.as_str() {
            Some(s) if app_id(s) => Ok(value.clone()),
            _ => Err(Fault::new(
                name,
                "not an app id",
                "use an iOS bundle id or Android package: letters, digits, `.`, `_`, `-`",
            )),
        },
        Kind::Path => match value.as_str() {
            Some(s) if !s.is_empty() => Ok(value.clone()),
            _ => Err(Fault::new(
                name,
                "must be a file path",
                "give a non-empty path",
            )),
        },
        Kind::Point => point(name, value),
        Kind::RefThenText => Err(Fault::new(
            name,
            "is not a single value",
            "give ref and text separately",
        )),
    }
}

fn parse_ref(name: &str, value: &Value) -> Result<Value, Fault> {
    value
        .as_str()
        .and_then(|s| Ref::parse(s).ok())
        .map(|r| json!(r.to_string()))
        .ok_or_else(|| Fault::new(name, "not a ref", REF_FIX))
}

fn number(
    name: &str,
    value: &Value,
    gt: Option<f64>,
    max: Option<f64>,
    except: Option<f64>,
) -> Result<Value, Fault> {
    let range = match (gt, max) {
        (Some(lo), Some(hi)) => format!("give a number above {lo} and at most {hi}"),
        (Some(lo), None) => format!("give a number above {lo}"),
        (None, Some(hi)) => format!("give a number at most {hi}"),
        (None, None) => "give a finite number".to_owned(),
    };
    match value.as_f64() {
        Some(n) if n.is_finite() && gt.is_none_or(|lo| n > lo) && max.is_none_or(|hi| n <= hi) => {
            Ok(json!(n))
        }
        Some(_) => Err(Fault::new(name, "out of range", range)),
        None => Err(Fault::new(name, "must be a finite number", range)),
    }
    .and_then(|n| match (except, n.as_f64()) {
        (Some(e), Some(v)) if (v - e).abs() < 0.01 => Err(Fault::new(
            name,
            format!("too close to {e}"),
            format!("give a value at least 0.01 away from {e}"),
        )),
        _ => Ok(n),
    })
}

fn point(name: &str, value: &Value) -> Result<Value, Fault> {
    const FIX: &str = "give {\"ref\": \"@snapshot:e1\"} or {\"x\": 12, \"y\": 34}";
    let Some(object) = value.as_object() else {
        return Err(Fault::new(name, "must be an object", FIX));
    };
    if let Some(key) = object
        .keys()
        .find(|k| !matches!(k.as_str(), "ref" | "x" | "y"))
    {
        return Err(Fault::new(&format!("{name}.{key}"), "unknown field", FIX));
    }
    match (object.get("ref"), object.get("x"), object.get("y")) {
        (Some(reference), None, None) => {
            Ok(json!({"ref": parse_ref(&format!("{name}.ref"), reference)?}))
        }
        (None, Some(x), Some(y)) => {
            let coordinate = |axis: &str, v: &Value| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .map(|n| json!(n))
                    .ok_or_else(|| {
                        Fault::new(&format!("{name}.{axis}"), "must be a finite number", FIX)
                    })
            };
            Ok(json!({"x": coordinate("x", x)?, "y": coordinate("y", y)?}))
        }
        (None, Some(_), None) => Err(Fault::new(&format!("{name}.y"), "missing", FIX)),
        (None, None, Some(_)) => Err(Fault::new(&format!("{name}.x"), "missing", FIX)),
        (None, None, None) => Err(Fault::new(name, "is empty", FIX)),
        (Some(_), ..) => Err(Fault::new(name, "has both a ref and a point", FIX)),
    }
}

fn app_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 1024
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The JSON value for one CLI argument's words, before [`check_args`].
/// Numbers that do not parse stay strings so the check names the field.
#[must_use]
pub fn from_words(kind: Kind, words: &[String]) -> Value {
    let number = |w: &String| w.parse::<f64>().map_or_else(|_| json!(w), |n| json!(n));
    match (kind, words) {
        (Kind::Point, [reference]) => json!({"ref": reference}),
        (Kind::Point, [x, y]) => json!({"x": number(x), "y": number(y)}),
        (Kind::Number { .. }, [w]) => number(w),
        (Kind::Integer, [w]) => w.parse::<u64>().map_or_else(|_| json!(w), |n| json!(n)),
        (_, [w]) => json!(w),
        _ => json!(words),
    }
}

/// Split CLI `[REF] TEXT...`: a leading ref-shaped word selects the field
/// when text follows it; otherwise every word is text.
#[must_use]
pub fn split_ref_text(words: &[String]) -> (Option<&str>, String) {
    match words.split_first() {
        Some((first, rest)) if !rest.is_empty() && Ref::parse(first).is_ok() => {
            (Some(first.as_str()), rest.join(" "))
        }
        _ => (None, words.join(" ")),
    }
}

#[cfg(test)]
#[path = "kind_tests.rs"]
mod tests;
