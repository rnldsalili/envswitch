//! Command-line layer (contract 01). Owner: C4 (`update.rs`: D3).
//! Public signatures and re-exports frozen by S1. `version_line` is
//! implemented by S1 and must stay byte-identical (D9).

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

pub mod args;
pub mod commands;
pub mod guided;
pub mod help;
pub mod output;
pub mod update;

#[cfg(windows)]
use ctrlc as _;
#[cfg(windows)]
use enable_ansi_support as _;
// ^ dependency anchors for the stub: remove when implemented.
use crate::ports::Deps;

pub use args::{FlagValue, ParsedArgs, parse_args};
pub use output::format_progress_event;

/// TS `main(argv, options)` (01 §0): dispatch, returns the exit code and never
/// exits the process. Errors print `envswitch: <msg>` to `deps.err` (1 or 2);
/// `Error::Cancelled` prints nothing (130). `argv` excludes the program name.
pub fn run(argv: Vec<String>, deps: &mut Deps) -> i32 {
    todo!("C4: 01 §0, §4.5")
}

/// One-time process setup called by `main.rs` before [`run`] and never by
/// in-process tests: on Windows, enable VT processing for stderr
/// (`enable-ansi-support`) and install a `ctrlc` handler that exits 130
/// outside prompts (DIVERGENCES b1). No-op work on other platforms.
pub fn init_process() {
    todo!("C4: 01 §4.2, §5.4; DIVERGENCES b1")
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
