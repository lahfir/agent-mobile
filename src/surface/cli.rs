//! The clap projection: the command tree is built from [`VERBS`], and
//! parsed words become the same [`Args`] the MCP surface produces.

use clap::builder::PossibleValuesParser;
use clap::{Arg as ClapArg, ArgAction, ArgMatches, Command};
use serde_json::{Map, Value, json};

use agent_mobile_core::contract::Ref;
use agent_mobile_core::error::Failure;

use super::kind::{self, Fault};
use super::{Arg, GLOBALS, Kind, VERBS, Verb, find};
use crate::cmd::Call;

const JSON: &str = "global-json";

fn global_id(name: &str) -> String {
    format!("global-{name}")
}

fn long(name: &str) -> String {
    name.replace('_', "-")
}

/// The full command tree.
#[must_use]
pub fn command() -> Command {
    let mut root = Command::new("agent-mobile")
        .version(env!("CARGO_PKG_VERSION"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .subcommand_required(true)
        .arg(
            ClapArg::new(JSON)
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Emit the raw JSON envelope instead of text"),
        );
    for global in GLOBALS {
        let arg = global.arg();
        root = root.arg(
            ClapArg::new(global_id(arg.name))
                .long(long(arg.name))
                .global(true)
                .value_name(arg.name.to_uppercase())
                .help(arg.help),
        );
    }
    for verb in VERBS {
        root = root.subcommand(subcommand(verb));
    }
    root
}

fn subcommand(verb: &Verb) -> Command {
    let mut sub = Command::new(verb.name).about(verb.summary);
    if !verb.guide.is_empty() {
        sub = sub.long_about(format!("{}\n\n{}", verb.summary, verb.guide));
    }
    for arg in verb.args {
        sub = sub.arg(clap_arg(arg));
    }
    sub
}

fn clap_arg(arg: &Arg) -> ClapArg {
    let mut out = ClapArg::new(arg.name)
        .help(arg.help)
        .required(arg.required)
        .value_name(arg.name.to_uppercase());
    if arg.flag {
        out = out.long(long(arg.name));
    }
    match arg.kind {
        Kind::Point => out.num_args(1..=2).value_name("REF | X Y"),
        Kind::RefThenText => out.num_args(1..).value_name("[REF] TEXT"),
        Kind::Number { .. } => out.allow_negative_numbers(true),
        Kind::Enum(values) => out.value_parser(PossibleValuesParser::new(values)),
        _ => out,
    }
}

/// Parse the process arguments. Clap faults exit 2 inside clap; the
/// returned name and `--json` flag shape the failure report.
pub fn parse() -> (Result<Call, Failure>, &'static str, bool) {
    let matches = command().get_matches();
    let json = matches.get_flag(JSON);
    let Some((name, sub)) = matches.subcommand() else {
        return (Err(Failure::usage("name a command; see --help")), "", json);
    };
    let Some(verb) = find(name) else {
        return (
            Err(Failure::usage(format!("unknown command {name}"))),
            "",
            json,
        );
    };
    (call(verb, sub, json), verb.name, json)
}

fn call(verb: &'static Verb, sub: &ArgMatches, json: bool) -> Result<Call, Failure> {
    let mut input = Map::new();
    for arg in verb.args {
        let Some(words) = sub.get_many::<String>(arg.name) else {
            continue;
        };
        let words: Vec<String> = words.cloned().collect();
        if arg.kind == Kind::RefThenText {
            ref_then_text(&words, &mut input).map_err(|f| f.usage())?;
        } else {
            input.insert(arg.name.to_owned(), kind::from_words(arg.kind, &words));
        }
    }
    for global in GLOBALS {
        let arg = global.arg();
        let Some(raw) = sub.get_one::<String>(&global_id(arg.name)) else {
            continue;
        };
        if verb.globals.contains(&global) {
            input.insert(
                arg.name.to_owned(),
                kind::from_words(arg.kind, std::slice::from_ref(raw)),
            );
        } else {
            eprintln!("note: --{} is ignored by {}", long(arg.name), verb.name);
        }
    }
    let args = kind::check_args(verb, &input, true).map_err(|f| f.usage())?;
    Ok(Call {
        verb,
        args,
        json,
        remember: true,
    })
}

fn ref_then_text(words: &[String], input: &mut Map<String, Value>) -> Result<(), Fault> {
    if let [only] = words
        && Ref::parse(only).is_ok()
    {
        return Err(Fault {
            field: "text".to_owned(),
            problem: "missing after the ref".to_owned(),
            fix: "quote the text, or use `--` if it begins with `-`".to_owned(),
        });
    }
    let (reference, text) = kind::split_ref_text(words);
    if let Some(reference) = reference {
        input.insert("ref".to_owned(), json!(reference));
    }
    input.insert("text".to_owned(), Value::String(text));
    Ok(())
}
