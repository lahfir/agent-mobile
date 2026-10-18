//! The command list in `skills/agent-mobile/SKILL.md`, projected from the registry.

use std::fmt::Write;

use super::{Arg, GLOBALS, Kind, VERBS};

/// Marks the generated block inside SKILL.md.
pub const START: &str = "<!-- commands:start -->\n";
/// Ends the generated block inside SKILL.md.
pub const END: &str = "<!-- commands:end -->";

fn arg_usage(arg: &Arg) -> String {
    if arg.flag {
        return format!("[--{} <{}>]", arg.name.replace('_', "-"), arg.name);
    }
    let shape = match arg.kind {
        Kind::Point => "<ref> | <x> <y>".to_owned(),
        Kind::RefThenText => "[<ref>] <text...>".to_owned(),
        Kind::Enum(values) if values.len() == 1 => values[0].to_owned(),
        _ => format!("<{}>", arg.name),
    };
    if arg.required {
        shape
    } else {
        format!("[{shape}]")
    }
}

/// One markdown line per verb, then the global flags.
#[must_use]
pub fn command_list() -> String {
    let mut verbs = String::new();
    for verb in VERBS {
        let mut usage = vec![verb.name.to_owned()];
        usage.extend(verb.args.iter().map(arg_usage));
        let _ = writeln!(verbs, "- `{}`: {}", usage.join(" "), verb.summary);
    }
    let flags: Vec<String> = GLOBALS
        .iter()
        .map(|global| format!("`--{}`", global.arg().name.replace('_', "-")))
        .collect();
    format!("{verbs}\nGlobal flags: {}, `--json`.\n", flags.join(", "))
}
