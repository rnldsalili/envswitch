//! Classification, merge, and the switch flow (contract 02 §4, §7). Owner: C3.
//! Public signatures frozen by S1; `matcher`/`merge` are C3-internal
//! (`pub(crate)`) and C3 may reshape them.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

pub mod matcher;
pub mod merge;

use std::path::Path;

use crate::error::Result;
use crate::model::{
    ClassifyParams, Config, InspectReport, ItemSummary, LayerName, ProgressEvent, SwitchRequest,
    SwitchResult,
};
use crate::ports::OnePassword;

/// TS `switchEnvironment(config, op, options)` (02 §4.5).
///
/// Event order (02 §7): `info validating project root <root>`, root markers,
/// selection, targets, resolve all selected targets (before any `op` call),
/// `info listing candidate 1Password items`, one `list_items_by_tags`,
/// `done found <n> candidate item(s)`, then per target `info processing
/// <name> -> <path>`, `info reading <title||id>` per read-cache miss (cache
/// shared across targets, reads sequential), and either `info writing <path>`,
/// write, `done wrote <path>` (path re-resolved right before writing) or
/// `dry-run would write <path>`. Never prints; emits only through `log`.
pub fn switch_environment(
    op: &dyn OnePassword,
    req: &SwitchRequest,
    log: &mut dyn FnMut(ProgressEvent),
) -> Result<SwitchResult> {
    todo!("C3: 02 §4.5, §7")
}

/// TS `inspectProject(config, root)` (02 §4.6): root markers, then resolve
/// every config target; `parent_exists` = parent dir exists (follows
/// symlinks, errors → false). No events, no `op` calls.
pub fn inspect_project(config: &Config, root: &Path) -> Result<InspectReport> {
    todo!("C3: 02 §4.6")
}

/// TS `classifyItemForTarget(config, item, params)` (02 §4.1–4.2): rebuilds
/// the matcher on every call. `cwd` is the `{project}` fallback source.
/// `None` = the item does not apply to this target.
pub fn classify_item_for_target(
    config: &Config,
    item: &ItemSummary,
    params: &ClassifyParams,
    cwd: &Path,
) -> Option<LayerName> {
    todo!("C3: 02 §4.1-4.2")
}
