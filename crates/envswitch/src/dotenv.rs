//! `.env` serialization and atomic write (contract 02 §6). Owner: C2.
//! Signatures frozen by S1.

use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use crate::error::{Error, Result, io_error, io_error_dest};
use crate::jsutil::{compare_utf16, is_env_key, js_is_space};
use crate::model::EnvMap;

/// TS `stringifyEnv(values)` (02 §6.1): keys sorted by UTF-16 code unit,
/// validated in sorted order (`Error::Internal("invalid environment variable
/// key: <key>")`, exit 2), `KEY=value` lines joined with `\n`, always a final
/// `\n` (an empty map gives `"\n"`).
pub fn stringify_env(values: &EnvMap) -> Result<String> {
    let mut keys: Vec<_> = values.keys().collect();
    keys.sort_by(|left, right| compare_utf16(left, right));
    let mut lines = Vec::with_capacity(keys.len());
    for key in keys {
        if !is_env_key(key) {
            return Err(Error::internal(format!(
                "invalid environment variable key: {key}"
            )));
        }
        let value = &values[key];
        let formatted = if value
            .chars()
            .any(|c| js_is_space(c) || matches!(c, '#' | '"' | '\'' | '`' | '$' | '\\'))
        {
            serde_json::to_string(value).map_err(|error| Error::internal(error.to_string()))?
        } else {
            value.clone()
        };
        lines.push(format!("{key}={formatted}"));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

/// TS `writeEnvFile(path, values)` (02 §6.2): serialize first, `mkdir -p`
/// (0o750 on unix), temp file `.<base>.<pid>.<uuid v4>.tmp` (0o600), rename
/// over `path`, remove the temp file on failure (cleanup errors take precedence),
/// then chmod 0o600. io errors
/// use `crate::error::io_error` (DIVERGENCES d1).
pub fn write_env_file(path: &Path, values: &EnvMap) -> Result<()> {
    let content = stringify_env(values)?;
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let temp = dir.join(name);
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o750);
    builder.create(dir).map_err(|error| {
        #[cfg(windows)]
        if error.kind() == std::io::ErrorKind::AlreadyExists && !dir.is_dir() {
            return io_error(
                &std::io::Error::from(std::io::ErrorKind::NotADirectory),
                "mkdir",
                dir,
            );
        }
        io_error(&error, "mkdir", dir)
    })?;

    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&temp)
            .map_err(|error| io_error(&error, "open", &temp))?;
        file.write_all(content.as_bytes())
            .map_err(|error| io_error(&error, "write", &temp))?;
        drop(file);
        chmod_private(&temp)?;
        fs::rename(&temp, path).map_err(|error| io_error_dest(&error, "rename", &temp, path))
    })();
    if result.is_err()
        && let Err(error) = fs::remove_file(&temp)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(io_error(&error, "rm", &temp));
    }
    result?;
    chmod_private(path)
}

fn chmod_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| io_error(&error, "chmod", path))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
