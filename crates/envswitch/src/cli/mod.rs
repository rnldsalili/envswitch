//! Command-line layer (contract 01). Owner: C4 (`update.rs`: D3).
//! Public signatures and re-exports frozen by S1. `version_line` is
//! implemented by S1 and must stay byte-identical (D9).

pub mod args;
pub mod commands;
pub mod guided;
pub mod help;
pub mod output;
pub mod update;

use crate::error::{Error, Result, fail};
use crate::ports::Deps;
use std::io::Write;

pub use args::{FlagValue, ParsedArgs, parse_args};
pub use output::format_progress_event;

/// TS `main(argv, options)` (01 §0): dispatch, returns the exit code and never
/// exits the process. Errors print `envswitch: <msg>` to `deps.err` (1 or 2);
/// `Error::Cancelled` prints nothing (130). `argv` excludes the program name.
pub fn run(argv: Vec<String>, deps: &mut Deps) -> i32 {
    match dispatch(&argv, deps) {
        Ok(()) => 0,
        Err(error) => {
            if error != Error::Cancelled {
                let _ = writeln!(deps.err, "envswitch: {error}");
            }
            error.exit_code()
        }
    }
}

/// One-time process setup called by `main.rs` before [`run`] and never by
/// in-process tests: on Windows, enable VT processing for stderr
/// (`enable-ansi-support`) and install a `ctrlc` handler that exits 130
/// outside prompts (DIVERGENCES b1). No-op work on other platforms.
pub fn init_process() {
    #[cfg(windows)]
    {
        let _ = enable_ansi_support::enable_ansi_support();
        let _ = ctrlc::set_handler(|| std::process::exit(130));
    }
}

/// `--version` output (D9, DIVERGENCES f1): `envswitch <version> <target>\n`,
/// e.g. `envswitch 0.0.42 aarch64-apple-darwin\n`. Written to stdout, exit 0.
pub fn version_line() -> String {
    format!(
        "envswitch {} {}\n",
        env!("CARGO_PKG_VERSION"),
        env!("TARGET")
    )
}

fn dispatch(argv: &[String], deps: &mut Deps) -> Result<()> {
    if argv.is_empty() {
        return guided::run_guided(deps);
    }
    if matches!(argv[0].as_str(), "--help" | "-h") {
        return deps
            .out
            .write_all(help::help_text().as_bytes())
            .map_err(|e| Error::internal(e.to_string()));
    }
    let args = parse_args(argv);
    if args.flags.get("help") == Some(&FlagValue::True) || args.command.as_deref() == Some("help") {
        return deps
            .out
            .write_all(help::help_text().as_bytes())
            .map_err(|e| Error::internal(e.to_string()));
    }
    match args.command.as_deref().unwrap_or_default() {
        "--version" => deps
            .out
            .write_all(version_line().as_bytes())
            .map_err(|e| Error::internal(e.to_string())),
        "switch" => commands::run_switch(&args, deps),
        "list" => commands::run_list(&args, deps),
        "doctor" => commands::run_doctor(&args, deps),
        "update" => {
            commands::validate_flags(&args, &["help"])?;
            update::run_update(&args, deps)
        }
        command => fail(format!("unknown command: {command}")),
    }
}
