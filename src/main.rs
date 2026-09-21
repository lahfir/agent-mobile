//! `agent-mobile` command-line entry point: parse, dispatch, map the failure
//! render to stderr, and exit with the registry code.

mod cli;
mod cmd;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let code = match cmd::dispatch(&cli) {
        Ok(code) => code,
        Err(f) if cli.json => f.report_json(cli.command.name()),
        Err(f) => f.report(),
    };
    std::process::exit(code);
}
