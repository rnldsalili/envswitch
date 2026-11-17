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

#[cfg(windows)]
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// TS `main(argv, options)` (01 §0): dispatch, returns the exit code and never
/// exits the process. Errors print `envswitch: <msg>` to `deps.err` (1 or 2);
/// `Error::Cancelled` prints nothing (130). `argv` excludes the program name.
pub fn run(argv: Vec<String>, deps: &mut Deps) -> i32 {
    match dispatch(&argv, deps) {
        Ok(()) => 0,
        Err(error) => {
            #[cfg(windows)]
            let interrupted = INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst);
            #[cfg(not(windows))]
            let interrupted = false;
            report_error(error, deps.err.as_mut(), interrupted)
        }
    }
}

fn report_error(error: Error, err: &mut dyn Write, interrupted: bool) -> i32 {
    if interrupted || error == Error::Cancelled {
        return 130;
    }
    let _ = writeln!(err, "envswitch: {error}");
    error.exit_code()
}

/// One-time process setup called by `main.rs` before [`run`] and never by
/// in-process tests: on Windows, enable VT processing for stderr
/// (`enable-ansi-support`) and install a Ctrl-C / Ctrl-Break handler that exits 130
/// outside prompts (DIVERGENCES b1). No-op work on other platforms.
pub fn init_process() {
    #[cfg(windows)]
    {
        let _ = enable_ansi_support::enable_ansi_support();
        let _ = ctrlc::set_handler(|| {
            if !crate::install::process::handoff_active() {
                INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
                std::process::exit(130);
            }
        });
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
        let _ = deps.out.write_all(help::help_text().as_bytes());
        return Ok(());
    }
    let args = parse_args(argv);
    if args.flags.get("help") == Some(&FlagValue::True) || args.command.as_deref() == Some("help") {
        let _ = deps.out.write_all(help::help_text().as_bytes());
        return Ok(());
    }
    match args.command.as_deref().unwrap_or_default() {
        "--version" => {
            let _ = deps.out.write_all(version_line().as_bytes());
            Ok(())
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interruption_suppresses_errors_and_overrides_their_exit_codes() {
        for error in [
            Error::user("bad input"),
            Error::internal("op failed"),
            Error::status(7, "installer failed"),
        ] {
            let mut err = Vec::new();
            assert_eq!(report_error(error.clone(), &mut err, true), 130);
            assert!(err.is_empty());
            assert_eq!(
                report_error(error.clone(), &mut err, false),
                error.exit_code()
            );
            assert_eq!(
                String::from_utf8(err).unwrap(),
                format!("envswitch: {error}\n")
            );
        }
    }
}
