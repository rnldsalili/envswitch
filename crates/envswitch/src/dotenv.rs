//! `.env` serialization and atomic write (contract 02 §6). Owner: C2.
//! Signatures frozen by S1.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use uuid as _;
// ^ dependency anchor for the stub: remove when implemented.
use std::path::Path;

use crate::error::Result;
use crate::model::EnvMap;

/// TS `stringifyEnv(values)` (02 §6.1): keys sorted by UTF-16 code unit,
/// validated in sorted order (`Error::Internal("invalid environment variable
/// key: <key>")`, exit 2), `KEY=value` lines joined with `\n`, always a final
/// `\n` (an empty map gives `"\n"`).
pub fn stringify_env(values: &EnvMap) -> Result<String> {
    todo!("C2: 02 §6.1")
}

/// TS `writeEnvFile(path, values)` (02 §6.2): serialize first, `mkdir -p`
/// (0o750 on unix), temp file `.<base>.<pid>.<uuid v4>.tmp` (0o600), rename
/// over `path`, remove the temp file on failure, then chmod 0o600. io errors
/// use `crate::error::io_error` (DIVERGENCES d1).
pub fn write_env_file(path: &Path, values: &EnvMap) -> Result<()> {
    todo!("C2: 02 §6.2")
}
