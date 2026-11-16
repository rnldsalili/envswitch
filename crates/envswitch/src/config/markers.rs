//! Project root markers (contract 02 §1.6). Owner: C1. Signature frozen by S1.

use std::fs;
use std::io::Read;
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
        if fs::metadata(&package_json_path).is_err() {
            return fail(
                "project root marker packageJsonName requires package.json in the current directory",
            );
        }
        let package_json = read_package_json(&package_json_path).map_err(|message| {
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
///
/// Like `readFile`, it opens first (so an unreadable path reports the `open`
/// error, even for a directory) and only then fails a directory on `read`.
fn read_package_json(path: &Path) -> std::result::Result<serde_json::Value, String> {
    let mut file = open_for_read(path).map_err(|err| node_io_message(&err, "open", path, None))?;
    if file.metadata().is_ok_and(|meta| meta.is_dir()) {
        // Unix `read(2)` on a directory and Windows `ReadFile` on a
        // directory handle both fail; Node reports EISDIR without a path.
        return Err("EISDIR: illegal operation on a directory, read".to_string());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|err| {
        // Node's `read` errors carry no path.
        let message = node_io_message(&err, "read", path, None);
        let suffix = format!(" '{}'", path.display());
        message
            .strip_suffix(&suffix)
            .map_or_else(|| message.clone(), str::to_string)
    })?;
    let text = String::from_utf8_lossy(&bytes);
    // The engine-specific suffix differs from Bun (DIVERGENCES a5, a8).
    serde_json::from_str(&text).map_err(|err| err.to_string())
}

/// `open(path, O_RDONLY)` the way libuv does it; on Windows that includes
/// `FILE_FLAG_BACKUP_SEMANTICS`, so a directory opens and fails on read.
fn open_for_read(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    }
    options.open(path)
}
