//! `OnePasswordClient` (contract 02 §5.2–5.4). Owner: C2.

// Stub file: remove when implemented.
#![allow(unused_variables, dead_code, clippy::todo)]

use std::sync::Arc;

use crate::error::Result;
use crate::model::{EnvMap, ItemSummary, Vault};
use crate::ports::{CommandRunner, OnePassword, OpFactory, VaultLister};

/// Client bound to one vault. Non-zero `op` exits become
/// `Error::Internal(stderr.trim() || "Command failed: op <args joined by space>")`;
/// stdout is never included in errors. JSON-shape problems are `Error::User`.
pub struct OnePasswordClient {
    vault: String,
    runner: Arc<dyn CommandRunner>,
}

impl OnePasswordClient {
    /// TS `new OnePasswordClient(vault, runner)`; no I/O.
    pub fn new(vault: impl Into<String>, runner: Arc<dyn CommandRunner>) -> Self {
        OnePasswordClient {
            vault: vault.into(),
            runner,
        }
    }

    /// The vault passed to `--vault`.
    pub fn vault(&self) -> &str {
        &self.vault
    }
}

impl OnePassword for OnePasswordClient {
    fn assert_authenticated(&self) -> Result<()> {
        todo!("C2: 02 §5.2")
    }

    fn list_items_by_tags(&self, tags: &[String]) -> Result<Vec<ItemSummary>> {
        todo!("C2: 02 §5.2-5.3")
    }

    fn read_config_item(&self, item: &str) -> Result<String> {
        todo!("C2: 02 §5.4")
    }

    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap> {
        todo!("C2: 02 §5.4")
    }
}

/// TS static `OnePasswordClient.listVaults(runner)`:
/// `op vault list --format json` (02 §5.2–5.3).
pub fn list_vaults(runner: &dyn CommandRunner) -> Result<Vec<Vault>> {
    todo!("C2: 02 §5.2-5.3")
}

/// Real [`OpFactory`]: one [`OnePasswordClient`] per call, sharing a runner.
pub struct OpClientFactory {
    runner: Arc<dyn CommandRunner>,
}

impl OpClientFactory {
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        OpClientFactory { runner }
    }
}

impl OpFactory for OpClientFactory {
    fn create(&self, vault: &str) -> Arc<dyn OnePassword> {
        Arc::new(OnePasswordClient::new(vault, Arc::clone(&self.runner)))
    }
}

/// Real [`VaultLister`]: delegates to [`list_vaults`].
pub struct OpVaultLister {
    runner: Arc<dyn CommandRunner>,
}

impl OpVaultLister {
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        OpVaultLister { runner }
    }
}

impl VaultLister for OpVaultLister {
    fn list_vaults(&self) -> Result<Vec<Vault>> {
        list_vaults(self.runner.as_ref())
    }
}
