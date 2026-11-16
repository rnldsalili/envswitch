//! `envswitch update` (contract 01 §6; DIVERGENCES e1–e4). Owner: D3.
//! Everything here is `pub(crate)` and D3 may reshape it freely. C4 calls
//! `run_update` from dispatch; flag validation (`unknown flag`) happens
//! before any receipt, network, or installer work (01 §9 row 23).

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use crate::error::Result;
use crate::ports::Deps;

use super::args::ParsedArgs;

/// TS `runUpdate` (01 §6), redesigned per DIVERGENCES e1–e4.
pub(crate) fn run_update(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    todo!("D3: 01 §6")
}
