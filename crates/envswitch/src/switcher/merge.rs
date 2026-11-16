//! Layer merge and title collation (contract 02 §4.3; D8). Owner: C3.
//! Everything here is `pub(crate)` and C3 may reshape it freely.

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use icu_collator as _;
use icu_locale as _;
// ^ dependency anchors for the stub: remove when implemented.
use std::cmp::Ordering;
use std::collections::HashMap;

use crate::error::Result;
use crate::model::{EnvMap, ItemSummary, MergeResult, ProgressEvent, ResolvedTarget};
use crate::ports::OnePassword;

use super::matcher::TagMatcher;

/// `left.localeCompare(right)` with a fixed `en-US` ICU collator
/// (`icu_collator`, default options; D8).
pub(crate) fn compare_titles(left: &str, right: &str) -> Ordering {
    todo!("C3: 02 §4.3, D8")
}

/// TS `mergeTarget(op, candidates, params)`. `cache` is keyed by
/// `ItemSummary::key()` and shared across targets within one switch.
pub(crate) fn merge_target(
    op: &dyn OnePassword,
    candidates: &[ItemSummary],
    resolved: &ResolvedTarget,
    matcher: &TagMatcher,
    cache: &mut HashMap<String, EnvMap>,
    log: &mut dyn FnMut(ProgressEvent),
) -> Result<MergeResult> {
    todo!("C3: 02 §4.3")
}
