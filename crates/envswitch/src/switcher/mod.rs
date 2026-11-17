//! Classification, merge, and the switch flow (contract 02 §4, §7). Owner: C3.
//! Public signatures frozen by S1; `matcher`/`merge` are C3-internal
//! (`pub(crate)`).

pub mod matcher;
pub mod merge;

#[cfg(test)]
mod collation_golden;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;

use crate::config::{
    candidate_list_tags, resolve_output_path, resolve_targets, select_targets,
    validate_project_root, validate_selection,
};
use crate::dotenv::write_env_file;
use crate::error::Result;
use crate::model::{
    ClassifyParams, Config, InspectReport, InspectedTarget, ItemSummary, LayerName, ProgressEvent,
    SwitchRequest, SwitchResult, TargetSwitchResult,
};
use crate::ports::OnePassword;

use matcher::{build_tag_matcher, classify_with_matcher};
use merge::{merge_target, planned_reads, with_reads};

/// TS `switchEnvironment(config, op, options)` (02 §4.5).
///
/// Event order (02 §7): `info validating project root <root>`, root markers,
/// selection, targets, resolve all selected targets (before any `op` call),
/// `info listing candidate 1Password items`, one `list_items_by_tags`,
/// `done found <n> candidate item(s)`, then per target `info processing
/// <name> -> <path>`, `info reading <title||id>` per read-cache miss (cache
/// shared across targets; reads may start early, DIVERGENCES l1), and
/// either `info writing <path>`, write, `done wrote <path>` (path re-resolved
/// right before writing) or
/// `dry-run would write <path>`. Never prints; emits only through `log`.
pub fn switch_environment(
    op: &dyn OnePassword,
    req: &SwitchRequest,
    log: &mut dyn FnMut(ProgressEvent),
) -> Result<SwitchResult> {
    let config = &req.config;
    log(ProgressEvent::info(format!(
        "validating project root {}",
        req.root.display()
    )));
    validate_project_root(config, &req.root)?;
    validate_selection(config, &req.env, &req.client)?;

    let selected_targets = select_targets(config, &req.target_name)?;
    let resolved_targets = resolve_targets(&req.root, &selected_targets)?;
    log(ProgressEvent::info("listing candidate 1Password items"));
    let candidates = op.list_items_by_tags(&candidate_list_tags(config, &req.env, &req.cwd))?;
    log(ProgressEvent::done(format!(
        "found {} candidate item(s)",
        candidates.len()
    )));
    let matcher = build_tag_matcher(config, &req.env, &req.client, &req.cwd);
    let mut cache = HashMap::new();
    // Every planned read may start early (DIVERGENCES l1); results, events,
    // and writes below still follow the sequential TS order.
    let plan = planned_reads(&candidates, &resolved_targets, &matcher);
    with_reads(op, &plan, |reads| {
        let mut targets = Vec::with_capacity(resolved_targets.len());
        for resolved in resolved_targets {
            log(ProgressEvent::info(format!(
                "processing {} -> {}",
                resolved.target.name,
                resolved.output_path.display()
            )));
            let merge = merge_target(reads, &candidates, &resolved, &matcher, &mut cache, log)?;

            if req.dry_run {
                log(ProgressEvent::dry_run(format!(
                    "would write {}",
                    resolved.output_path.display()
                )));
            } else {
                // Re-check right before writing: a parent may have been swapped
                // for a symlink while `op` was running (02 §2.2).
                let output_path = resolve_output_path(&req.root, &resolved.target.output)?;
                log(ProgressEvent::info(format!(
                    "writing {}",
                    output_path.display()
                )));
                write_env_file(&output_path, &merge.values)?;
                log(ProgressEvent::done(format!(
                    "wrote {}",
                    output_path.display()
                )));
            }

            targets.push(TargetSwitchResult {
                target: resolved.target,
                output_path: resolved.output_path,
                written: !req.dry_run,
                key_count: merge.values.len(),
                layers: merge.layers,
                overwritten_keys: merge.overwritten_keys,
            });
        }

        Ok(SwitchResult { targets })
    })
}

/// TS `inspectProject(config, root)` (02 §4.6): root markers, then resolve
/// every config target; `parent_exists` = parent dir exists (follows
/// symlinks, errors → false). No events, no `op` calls.
pub fn inspect_project(config: &Config, root: &Path) -> Result<InspectReport> {
    validate_project_root(config, root)?;
    let targets = resolve_targets(root, &config.targets)?
        .into_iter()
        .map(|resolved| {
            // Node `dirname` of an absolute path; `parent()` is only `None`
            // for a filesystem root, where `dirname` returns the root itself.
            let parent = resolved
                .output_path
                .parent()
                .unwrap_or(&resolved.output_path);
            InspectedTarget {
                parent_exists: parent.exists(),
                target: resolved.target,
                output_path: resolved.output_path,
            }
        })
        .collect();
    Ok(InspectReport { targets })
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
    let matcher = build_tag_matcher(config, &params.env, &params.client, cwd);
    classify_with_matcher(item, &params.target, &matcher)
}
