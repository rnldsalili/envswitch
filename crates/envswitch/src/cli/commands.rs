//! `switch`, `list`, `doctor` (contract 01 §2–3). Owner: C4.
//! Everything here is `pub(crate)` and C4 may reshape it freely.

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use crate::error::Result;
use crate::ports::Deps;

use super::args::ParsedArgs;

/// TS `runSwitch` (01 §3.1).
pub(crate) fn run_switch(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    todo!("C4: 01 §3.1")
}

/// TS `runList` (01 §3.2).
pub(crate) fn run_list(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    todo!("C4: 01 §3.2")
}

/// TS `runDoctor` (01 §3.3).
pub(crate) fn run_doctor(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    todo!("C4: 01 §3.3")
}
