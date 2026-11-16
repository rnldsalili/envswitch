//! `OnePasswordClient` (contract 02 §5.2–5.4). Owner: C2.

use std::sync::Arc;

use super::OP_PROGRAM;
use super::parse::{RawField, parse_item_fields, parse_item_list, parse_vault_list};
use crate::error::{Error, Result};
use crate::jsutil::{is_env_key, js_trim};
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

    fn get_raw_item(&self, item: &str) -> Result<Vec<RawField>> {
        let raw = run_op(
            self.runner.as_ref(),
            &[
                "item",
                "get",
                item,
                "--vault",
                &self.vault,
                "--format",
                "json",
                "--reveal",
            ],
        )?;
        parse_item_fields(&raw, item)
    }

    /// The vault passed to `--vault`.
    pub fn vault(&self) -> &str {
        &self.vault
    }
}

impl OnePassword for OnePasswordClient {
    fn assert_authenticated(&self) -> Result<()> {
        run_op(self.runner.as_ref(), &["whoami", "--format", "json"])?;
        Ok(())
    }

    fn list_items_by_tags(&self, tags: &[String]) -> Result<Vec<ItemSummary>> {
        let joined = tags.join(",");
        let mut args = vec!["item", "list", "--vault", &self.vault, "--format", "json"];
        if !tags.is_empty() {
            args.extend(["--tags", &joined]);
        }
        parse_item_list(&run_op(self.runner.as_ref(), &args)?)
    }

    fn read_config_item(&self, item: &str) -> Result<String> {
        let fields = self.get_raw_item(item)?;
        fields
            .into_iter()
            .find(|field| field.label == "config")
            .filter(|field| !js_trim(&field.value).is_empty())
            .map(|field| field.value)
            .ok_or_else(|| {
                Error::user(format!(
                    "1Password config item {item} must contain a non-empty field named config"
                ))
            })
    }

    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap> {
        let mut values = EnvMap::new();
        for field in self.get_raw_item(item.key())? {
            let label = js_trim(&field.label);
            if label.is_empty() || label == "notesPlain" || !is_env_key(label) {
                continue;
            }
            values.insert(label.to_owned(), field.value);
        }
        Ok(values)
    }
}

/// TS static `OnePasswordClient.listVaults(runner)`:
/// `op vault list --format json` (02 §5.2–5.3).
pub fn list_vaults(runner: &dyn CommandRunner) -> Result<Vec<Vault>> {
    parse_vault_list(&run_op(runner, &["vault", "list", "--format", "json"])?)
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

// Only stderr can become an error message: stdout may contain revealed secrets.
fn run_op(runner: &dyn CommandRunner, args: &[&str]) -> Result<String> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let output = runner.run(OP_PROGRAM, &args)?;
    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = js_trim(&stderr);
        return Err(Error::internal(if message.is_empty() {
            format!("Command failed: {OP_PROGRAM} {}", args.join(" "))
        } else {
            message.to_owned()
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
