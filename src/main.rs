//! `agent-mobile` command-line entry point: parse through the registry,
//! run, map the failure render to stderr, and exit with the registry code.

mod cmd;
mod mcp;
mod platform;
mod surface;

fn main() {
    let (call, name, json) = surface::cli::parse();
    let code = match call.and_then(|call| cmd::run_cli(&call)) {
        Ok(code) => code,
        Err(f) if json && name != "mcp" => f.report_json(name),
        Err(f) => f.report(),
    };
    std::process::exit(code);
}
