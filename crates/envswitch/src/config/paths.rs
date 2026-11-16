//! Output path safety (contract 02 §2). Owner: C1. Signature frozen by S1.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use std::path::{Path, PathBuf};

use crate::error::Result;

/// TS `resolveOutputPath(root, output)` (02 §2.1, §2.3).
///
/// * Lexical normalization only (never `canonicalize`). A relative `root` is
///   resolved against the process cwd, like Node `path.resolve`.
/// * `User` errors: `target output path cannot be empty`,
///   `... must be relative to the project root: <output>`,
///   `... escapes the project root: <output>` (string `..` prefix test kept,
///   so `..env` is rejected), `... contains a symbolic-link parent: <output>`.
/// * The lstat walk covers every ancestor from the output's parent up to and
///   including the root. `NotFound` continues; any other io error is
///   `crate::error::io_error(err, "lstat", path)` (exit 2, DIVERGENCES d1).
/// * Windows drive-relative paths keep parity (no tightening).
pub fn resolve_output_path(root: &Path, output: &str) -> Result<PathBuf> {
    todo!("C1: 02 §2")
}
