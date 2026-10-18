//! Usage strings for the skills guide, projected from the surface registry.

use super::{Arg, GLOBALS, Kind, Surface, VERBS};

/// Render the registry-owned commands and global flags in the agent guide layout.
#[must_use]
pub fn guide_sections() -> String {
    let arg_usage = |arg: &Arg| {
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
    };
    let wrapped_row = |usage: &str, description: &str| {
        let continued = "                                 ";
        let long_usage = usage.chars().count() > 31;
        let prefix = if long_usage {
            continued.to_owned()
        } else {
            format!("  {usage:<31}  ")
        };
        let mut result = if long_usage {
            format!("  {usage}\n")
        } else {
            String::new()
        };
        let mut line = prefix.clone();
        for word in description.split_whitespace() {
            let separator = usize::from(!line.ends_with(' '));
            if line.chars().count() + separator + word.chars().count() > 77 && line != prefix {
                result.push_str(line.trim_end());
                result.push('\n');
                line.clear();
                line.push_str(continued);
            }
            if !line.ends_with(' ') {
                line.push(' ');
            }
            line.push_str(word);
        }
        result.push_str(line.trim_end());
        result.push('\n');
        result
    };
    let mut output = String::from("COMMANDS\n");
    for verb in VERBS
        .iter()
        .filter(|verb| verb.surface == Surface::Both || verb.surface == Surface::Cli)
    {
        let mut parts = vec![verb.name.to_owned()];
        parts.extend(verb.args.iter().map(arg_usage));
        let usage = parts.join(" ");
        let description = if verb.guide.is_empty() {
            verb.summary.to_owned()
        } else {
            format!("{}. {}", verb.summary.trim_end_matches('.'), verb.guide)
        };
        output.push_str(&wrapped_row(&usage, &description));
    }
    output.push_str("\nFLAGS (global)\n");
    output.push_str(&wrapped_row(
        "--json",
        "Emit the raw JSON envelope instead of text; failures print the envelope shape on stdout too",
    ));
    for global in GLOBALS {
        let arg = global.arg();
        output.push_str(&wrapped_row(
            &format!("--{} <{}>", arg.name.replace('_', "-"), arg.name),
            arg.help,
        ));
    }
    output.push('\n');
    output
}
