//! Project root markers (contract 02 §1.6). Owner: C1. Signature frozen by S1.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use std::path::Path;

use crate::error::Result;
use crate::model::Config;

/// TS `validateProjectRoot(config, root)` (02 §1.6): every marker in order,
/// `path` (via `resolve_output_path`, target-flavored messages) before
/// `packageJsonName` (`join(root, "package.json")`). No markers → `Ok`.
pub fn validate_project_root(config: &Config, root: &Path) -> Result<()> {
    todo!("C1: 02 §1.6")
}
