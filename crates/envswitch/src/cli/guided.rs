//! Guided mode (contract 01 §5) and the real prompter. Owner: C4.
//! `InquirePrompter` is public (referenced by `ports::Deps` docs); the rest is
//! `pub(crate)` and C4 may reshape it.

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use inquire as _;
// ^ dependency anchor for the stub: remove when implemented.
use crate::error::Result;
use crate::ports::{Deps, InputSpec, Prompter, SelectSpec};

/// Real [`Prompter`] backed by `inquire` (renders on stderr, DIVERGENCES c1;
/// substring filter, c2; Esc and Ctrl-C → `Error::Cancelled`, c3). Select uses
/// page size 7, no wraparound, starting cursor = `default_index`.
#[derive(Debug, Clone, Copy, Default)]
pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    fn select(&self, spec: &SelectSpec) -> Result<usize> {
        todo!("C4: 01 §5.3-5.4")
    }

    fn input(&self, spec: &InputSpec) -> Result<String> {
        todo!("C4: 01 §5.3-5.4")
    }
}

/// TS `runGuidedSwitch(deps)` (01 §5.1–5.2). See `ports` for the vault-scan
/// thread rules.
pub(crate) fn run_guided(deps: &mut Deps) -> Result<()> {
    todo!("C4: 01 §5")
}
