//! Lenient parsing of `op` JSON output (contract 02 §5.3). Owner: C2.
//! Everything here is `pub(crate)` and C2 may reshape it freely.

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use crate::error::Result;
use crate::model::{ItemSummary, Vault};

/// One `fields[i]` entry: `label`/`value` strings, else `""`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RawField {
    pub label: String,
    pub value: String,
}

/// `op vault list` stdout → vaults (`name || id`).
pub(crate) fn parse_vault_list(raw: &str) -> Result<Vec<Vault>> {
    todo!("C2: 02 §5.3")
}

/// `op item list` stdout → items, API order preserved.
pub(crate) fn parse_item_list(raw: &str) -> Result<Vec<ItemSummary>> {
    todo!("C2: 02 §5.3")
}

/// `op item get <item>` stdout → fields, in order.
pub(crate) fn parse_item_fields(raw: &str, item: &str) -> Result<Vec<RawField>> {
    todo!("C2: 02 §5.3")
}
