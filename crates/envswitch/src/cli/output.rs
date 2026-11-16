//! Progress formatting and summaries (contract 01 §4). Owner: C4.

use std::io::Write;
use std::path::Path;

use crate::error::Result;
use crate::model::{ProgressEvent, ProgressStatus, SwitchResult};
use crate::ports::Deps;

/// TS `formatProgressEvent(event, { color })` (01 §4.2), no trailing newline.
/// Plain: `envswitch [<status>] <message>`. Color: hard-coded SGR bytes.
pub fn format_progress_event(event: &ProgressEvent, color: bool) -> String {
    if !color {
        return format!("envswitch [{}] {}", event.status, event.message);
    }
    let sgr = match event.status {
        ProgressStatus::Info => 34,
        ProgressStatus::Done => 32,
        ProgressStatus::DryRun => 33,
    };
    format!(
        "\x1b[2menvswitch\x1b[0m \x1b[{sgr}m[{}]\x1b[0m {}",
        event.status, event.message
    )
}

pub(crate) fn log_progress(deps: &mut Deps, event: ProgressEvent) {
    if let Some(log) = &mut deps.log {
        log(event);
    } else {
        let color = deps.stderr_is_tty
            && (deps.env)("CI").is_none_or(|value| value.is_empty())
            && (deps.env)("NO_COLOR").is_none();
        let _ = writeln!(deps.err, "{}", format_progress_event(&event, color));
    }
}

pub(crate) fn write_line(deps: &mut Deps, line: impl std::fmt::Display) -> Result<()> {
    let _ = writeln!(deps.out, "{line}");
    Ok(())
}

pub(crate) fn print_switch_results(
    results: &SwitchResult,
    root: &Path,
    dry_run: bool,
    deps: &mut Deps,
) -> Result<()> {
    let action = if dry_run { "Dry run" } else { "Updated" };
    write_line(
        deps,
        format!(
            "{action} {} target(s) from {}",
            results.targets.len(),
            crate::jsutil::node_basename(root)
        ),
    )?;
    for result in &results.targets {
        write_line(
            deps,
            format!(
                "- {} -> {}",
                result.target.name,
                result.output_path.display()
            ),
        )?;
        write_line(deps, format!("  keys: {}", result.key_count))?;
        let layers = result
            .layers
            .iter()
            .map(|layer| format!("{}:{}", layer.name, layer.item.title))
            .collect::<Vec<_>>()
            .join(", ");
        write_line(
            deps,
            format!(
                "  layers: {}",
                if layers.is_empty() { "none" } else { &layers }
            ),
        )?;
        if !result.overwritten_keys.is_empty() {
            write_line(
                deps,
                format!("  overwritten: {}", result.overwritten_keys.join(", ")),
            )?;
        }
    }
    Ok(())
}
