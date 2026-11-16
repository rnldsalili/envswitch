//! Real process runner (contract 02 §5.1). Owner: C2.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use crate::error::Result;
use crate::ports::{CommandOutput, CommandRunner};

/// TS `execFile` `maxBuffer`: stdout above this fails with
/// `stdout maxBuffer length exceeded`.
pub const MAX_STDOUT_BYTES: usize = 50 * 1024 * 1024;

/// `std::process::Command` runner: no shell, no timeout, inherited env and
/// cwd, stdin not a TTY, stdout capped at [`MAX_STDOUT_BYTES`].
/// Spawn failure → `Error::Internal("failed to run <program>: <io error>")`
/// (DIVERGENCES a6).
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecCommandRunner;

impl CommandRunner for ExecCommandRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        todo!("C2: 02 §5.1")
    }
}
