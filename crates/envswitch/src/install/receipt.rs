//! Install receipt (`install.json`, DIVERGENCES e3/e4). Owner: D3.
//! Provisional: the schema comes from `INSTALLER-PROTOCOL.md` (D2a).

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Install locations recorded by the installer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Receipt {
    pub app_root: PathBuf,
    pub install_dir: Option<PathBuf>,
    pub version: Option<String>,
}

/// Reads the receipt that belongs to the running executable. `Ok(None)` when
/// there is none (copied binary → reinstall guidance, e4).
pub(crate) fn read_receipt(current_exe: &Path) -> Result<Option<Receipt>> {
    todo!("D3: DIVERGENCES e3/e4")
}
