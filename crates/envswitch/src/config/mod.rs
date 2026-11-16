//! Config parsing and selection (contract 02 §1, §2, §3). Owner: C1.
//! Signatures and re-exports frozen by S1.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

pub mod markers;
pub mod paths;
pub mod tags;

use std::path::Path;

use crate::error::Result;
use crate::model::{Config, ResolvedTarget, TargetConfig};

pub use markers::validate_project_root;
pub use paths::resolve_output_path;
pub use tags::{TemplateContext, candidate_list_tags, compact_unique, render_template, tag_values};

/// TS `parseConfig(raw)` (02 §1.1–1.3). Manual `serde_json::Value` walk in
/// the exact TS check order; first failure wins; verbatim messages as
/// `Error::User`. JSON `null` is never "absent".
pub fn parse_config(raw: &str) -> Result<Config> {
    todo!("C1: 02 §1.1-1.3")
}

/// TS `validateSelection(config, env, client)` (02 §1.5): identifier checks
/// (env, then client), then membership when the list is present and
/// non-empty.
pub fn validate_selection(config: &Config, env: &str, client: &str) -> Result<()> {
    todo!("C1: 02 §1.5")
}

/// TS `selectTargets(config, targetName)` (02 §1.5): `"all"` returns every
/// target in config order without an identifier check; otherwise identifier
/// check, then the first exact match or `target <name> is not defined in
/// config.targets`.
pub fn select_targets(config: &Config, target_name: &str) -> Result<Vec<TargetConfig>> {
    todo!("C1: 02 §1.5")
}

/// TS `resolveTargets(root, targets)`: [`resolve_output_path`] for each
/// target in order; first failure wins.
pub fn resolve_targets(root: &Path, targets: &[TargetConfig]) -> Result<Vec<ResolvedTarget>> {
    todo!("C1: 02 §2")
}
