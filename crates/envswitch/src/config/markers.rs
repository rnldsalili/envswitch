//! Project root markers (contract 02 §1.6). Owner: C1. Signature frozen by S1.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result, fail, node_io_message};
use crate::model::{Config, RootMarker};

use super::paths::{native_join, resolve_output_path};

/// TS `validateProjectRoot(config, root)` (02 §1.6): every marker in order,
/// `path` (via `resolve_output_path`, target-flavored messages) before
/// `packageJsonName` (`join(root, "package.json")`). No markers → `Ok`.
pub fn validate_project_root(config: &Config, root: &Path) -> Result<()> {
    let markers = config
        .project
        .as_ref()
        .and_then(|project| project.root_markers.as_deref())
        .unwrap_or_default();
    for marker in markers {
        validate_marker(root, marker)?;
    }
    Ok(())
}

/// TS `validateMarker(root, marker)`. Both fields are tested for JS
/// truthiness, so an empty string is skipped like an absent one.
fn validate_marker(root: &Path, marker: &RootMarker) -> Result<()> {
    if let Some(path) = marker.path.as_deref().filter(|path| !path.is_empty()) {
        // Inherits the target-flavored messages (02 §9.3 #20).
        let marker_path = resolve_output_path(root, path)?;
        // `stat` follows symlinks; any error (EACCES, dangling link, ...)
        // counts as missing.
        if fs::metadata(&marker_path).is_err() {
            return fail(format!("project root marker path does not exist: {path}"));
        }
    }

    if let Some(name) = marker
        .package_json_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        let package_json_path = native_join(root, "package.json");
        let Ok(meta) = fs::metadata(&package_json_path) else {
            return fail(
                "project root marker packageJsonName requires package.json in the current directory",
            );
        };
        let package_json =
            read_package_json(&package_json_path, meta.is_dir()).map_err(|message| {
                Error::user(format!(
                    "failed to read package.json for project root validation: {message}"
                ))
            })?;
        let matches = package_json
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(|value| value.as_str())
            == Some(name);
        if !matches {
            return fail(format!(
                "current directory package.json name does not match {name}"
            ));
        }
    }
    Ok(())
}

/// `JSON.parse(await readFile(path, "utf8"))`; the error is the message the
/// TS catch block interpolates. `readFile` decodes invalid UTF-8 as U+FFFD
/// and keeps a leading BOM (which then fails to parse in both runtimes).
fn read_package_json(path: &Path, is_dir: bool) -> std::result::Result<serde_json::Value, String> {
    if is_dir {
        // Node opens the directory fine and fails on the first read.
        return Err("EISDIR: illegal operation on a directory, read".to_string());
    }
    let bytes = fs::read(path).map_err(|err| node_io_message(&err, "open", path, None))?;
    let text = String::from_utf8_lossy(&bytes);
    // The engine-specific suffix differs from Bun (DIVERGENCES a5).
    serde_json::from_str(&text).map_err(|err| err.to_string())
}
