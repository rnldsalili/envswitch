//! Native self-update support (03 §B7; DIVERGENCES e1–e5). Owner: D3.
//! **Provisional**: D3 finalizes these against `INSTALLER-PROTOCOL.md` (D2a).
//! The two public structs are referenced by `Deps::real()` and must keep
//! their names; everything else may change with orchestrator approval.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

pub mod embed;
pub mod receipt;

use crate::error::Result;
use crate::ports::{InstallPlan, InstallerRunner, ReleaseSource};

/// GitHub repository releases are published from.
pub const GITHUB_REPO: &str = "rnldsalili/envswitch";

/// Real [`ReleaseSource`] (provisional; no HTTP client crate is approved,
/// see PLAN §2.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct LatestReleaseSource;

impl ReleaseSource for LatestReleaseSource {
    fn latest_version(&self) -> Option<String> {
        todo!("D3: 03 §B7")
    }
}

/// Real [`InstallerRunner`]: writes the embedded installer to private temp
/// storage and runs it (`bash` / `powershell -NoProfile -ExecutionPolicy
/// Bypass -File`) with inherited stdio; signal death → `128 + n`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddedInstaller;

impl InstallerRunner for EmbeddedInstaller {
    fn run(&self, plan: &InstallPlan) -> Result<i32> {
        todo!("D3: 03 §B7, 01 §6")
    }
}
