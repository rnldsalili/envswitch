//! Tag matcher (contract 02 §3.5, §4.1–4.2). Owner: C3.
//! Everything here is `pub(crate)` and C3 may reshape it freely.

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use regex as _;
// ^ dependency anchor for the stub: remove when implemented.
use std::path::Path;

use crate::model::{Config, ItemSummary, LayerName};

/// Rendered into a scope template, then replaced with
/// `[^\r\n\u{2028}\u{2029}]+` in the scope regex (02 §3.5 erratum).
pub(crate) const SCOPE_SENTINEL: &str = "__ENVSWITCH_SCOPE__";

/// TS `TagMatcher`, built once per switch and shared by all targets.
pub(crate) struct TagMatcher {
    _private: (),
}

/// TS `buildTagMatcher(config, env, client)`.
pub(crate) fn build_tag_matcher(
    config: &Config,
    env: &str,
    client: &str,
    cwd: &Path,
) -> TagMatcher {
    todo!("C3: 02 §4.1")
}

/// TS `classifyItemWithMatcher(item, target, matcher)`.
pub(crate) fn classify_with_matcher(
    item: &ItemSummary,
    target: &str,
    matcher: &TagMatcher,
) -> Option<LayerName> {
    todo!("C3: 02 §4.2")
}
