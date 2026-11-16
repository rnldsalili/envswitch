//! 1Password CLI integration (contract 02 §5). Owner: C2.
//! Public items are frozen by S1; `pub(crate)` items in `parse` are C2's to
//! reshape.

pub mod client;
pub mod parse;
pub mod runner;

pub use client::{OnePasswordClient, OpClientFactory, OpVaultLister, list_vaults};
pub use runner::ExecCommandRunner;

/// The program every 1Password call runs (resolved through `PATH`; on
/// Windows `op.exe`). No `--account` flag is ever passed.
pub const OP_PROGRAM: &str = "op";
