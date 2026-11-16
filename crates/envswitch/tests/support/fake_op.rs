//! Thread-safe 1Password fakes: `FakeOp`, `FakeOpFactory`, `FakeVaultLister`,
//! and `ConcurrencyGate` (proves the guided vault scan runs concurrently).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use envswitch::error::{Error, Result};
use envswitch::model::{EnvMap, ItemSummary, Vault};
use envswitch::ports::{OnePassword, OpFactory, VaultLister};
use indexmap::IndexMap;

/// One recorded `OnePassword` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpCall {
    AssertAuthenticated,
    ListItemsByTags(Vec<String>),
    ReadConfigItem(String),
    ReadEnvItem(ItemSummary),
}

impl OpCall {
    /// The `op` argv (without the program name) the real client would run
    /// for this call against `vault` (contract 02 §5.2).
    pub fn argv(&self, vault: &str) -> Vec<String> {
        let owned = |parts: &[&str]| {
            parts
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        };
        match self {
            OpCall::AssertAuthenticated => owned(&["whoami", "--format", "json"]),
            OpCall::ListItemsByTags(tags) => {
                let mut argv = owned(&["item", "list", "--vault", vault, "--format", "json"]);
                if !tags.is_empty() {
                    argv.push("--tags".to_string());
                    argv.push(tags.join(","));
                }
                argv
            }
            OpCall::ReadConfigItem(item) => owned(&[
                "item", "get", item, "--vault", vault, "--format", "json", "--reveal",
            ]),
            OpCall::ReadEnvItem(item) => owned(&[
                "item",
                "get",
                item.key(),
                "--vault",
                vault,
                "--format",
                "json",
                "--reveal",
            ]),
        }
    }
}

/// Builds an `ItemSummary` from string slices.
pub fn item(id: &str, title: &str, tags: &[&str]) -> ItemSummary {
    ItemSummary {
        id: id.to_string(),
        title: title.to_string(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
    }
}

/// Builds an `EnvMap` from pairs, in order.
pub fn env_map(pairs: &[(&str, &str)]) -> EnvMap {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// Barrier with a timeout. Each caller of [`ConcurrencyGate::wait`] blocks
/// until `expected` callers have arrived or the timeout passes. A sequential
/// implementation therefore times out instead of deadlocking.
pub struct ConcurrencyGate {
    expected: usize,
    timeout: Duration,
    arrived: Mutex<usize>,
    cv: Condvar,
}

impl ConcurrencyGate {
    pub fn new(expected: usize, timeout: Duration) -> Arc<Self> {
        Arc::new(ConcurrencyGate {
            expected,
            timeout,
            arrived: Mutex::new(0),
            cv: Condvar::new(),
        })
    }

    /// Returns `true` when all `expected` callers arrived within the timeout.
    pub fn wait(&self) -> bool {
        let mut arrived = self.arrived.lock().unwrap();
        *arrived += 1;
        self.cv.notify_all();
        let (arrived, _) = self
            .cv
            .wait_timeout_while(arrived, self.timeout, |count| *count < self.expected)
            .unwrap();
        *arrived >= self.expected
    }

    pub fn arrived(&self) -> usize {
        *self.arrived.lock().unwrap()
    }
}

type ReadHook = Arc<dyn Fn(&ItemSummary) + Send + Sync>;

struct FakeOpState {
    calls: Vec<OpCall>,
    read_counts: IndexMap<String, usize>,
    auth: Result<()>,
    list_rules: Vec<(Vec<String>, Result<Vec<ItemSummary>>)>,
    default_list: Result<Vec<ItemSummary>>,
    config_items: HashMap<String, Result<String>>,
    env_values: HashMap<String, Result<EnvMap>>,
    on_read: Option<ReadHook>,
    gate: Option<Arc<ConcurrencyGate>>,
}

struct FakeOpInner {
    vault: String,
    state: Mutex<FakeOpState>,
}

/// Scripted [`OnePassword`] bound to one vault. Cheap to clone; clones share
/// state, so a test keeps a handle after moving one into `Deps`.
///
/// Defaults: `assert_authenticated` succeeds, `list_items_by_tags` returns
/// `[]`, `read_env_item` returns an empty map, and `read_config_item` fails
/// with an `Internal` error naming the unscripted item.
#[derive(Clone)]
pub struct FakeOp {
    inner: Arc<FakeOpInner>,
}

impl FakeOp {
    pub fn new(vault: &str) -> Self {
        FakeOp {
            inner: Arc::new(FakeOpInner {
                vault: vault.to_string(),
                state: Mutex::new(FakeOpState {
                    calls: Vec::new(),
                    read_counts: IndexMap::new(),
                    auth: Ok(()),
                    list_rules: Vec::new(),
                    default_list: Ok(Vec::new()),
                    config_items: HashMap::new(),
                    env_values: HashMap::new(),
                    on_read: None,
                    gate: None,
                }),
            }),
        }
    }

    pub fn vault(&self) -> &str {
        &self.inner.vault
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeOpState> {
        self.inner.state.lock().unwrap()
    }

    // -- configuration (builder style; clones share the change) -------------

    /// Items returned by `list_items_by_tags` for any tag list without a
    /// more specific rule.
    pub fn with_items(self, items: Vec<ItemSummary>) -> Self {
        self.state().default_list = Ok(items);
        self
    }

    /// Error returned by `list_items_by_tags` for any tag list without a more
    /// specific rule.
    pub fn with_list_error(self, error: Error) -> Self {
        self.state().default_list = Err(error);
        self
    }

    /// Response for an exact tag list (e.g. `["envswitch-config"]`).
    pub fn with_list_for(self, tags: &[&str], result: Result<Vec<ItemSummary>>) -> Self {
        let tags = tags.iter().map(|tag| tag.to_string()).collect();
        self.state().list_rules.push((tags, result));
        self
    }

    /// `read_config_item(item)` returns `raw` (the config field value).
    pub fn with_config(self, item: &str, raw: &str) -> Self {
        self.state()
            .config_items
            .insert(item.to_string(), Ok(raw.to_string()));
        self
    }

    pub fn with_config_error(self, item: &str, error: Error) -> Self {
        self.state()
            .config_items
            .insert(item.to_string(), Err(error));
        self
    }

    /// `read_env_item` for an item whose `key()` (`id || title`) is `key`.
    pub fn with_values(self, key: &str, values: EnvMap) -> Self {
        self.state().env_values.insert(key.to_string(), Ok(values));
        self
    }

    pub fn with_read_error(self, key: &str, error: Error) -> Self {
        self.state().env_values.insert(key.to_string(), Err(error));
        self
    }

    pub fn with_auth_error(self, error: Error) -> Self {
        self.state().auth = Err(error);
        self
    }

    /// Runs before every `read_env_item` result is returned (outside the
    /// lock), e.g. to swap a directory for a symlink mid-switch.
    pub fn on_read(self, hook: impl Fn(&ItemSummary) + Send + Sync + 'static) -> Self {
        self.state().on_read = Some(Arc::new(hook));
        self
    }

    /// Every `list_items_by_tags` call waits on `gate` first; a timeout
    /// becomes an `Internal` error.
    pub fn with_gate(self, gate: Arc<ConcurrencyGate>) -> Self {
        self.state().gate = Some(gate);
        self
    }

    // -- inspection ----------------------------------------------------------

    pub fn calls(&self) -> Vec<OpCall> {
        self.state().calls.clone()
    }

    /// `op` argv for every recorded call, in order.
    pub fn argvs(&self) -> Vec<Vec<String>> {
        let vault = self.vault().to_string();
        self.calls().iter().map(|call| call.argv(&vault)).collect()
    }

    /// Number of `read_env_item` calls per `item.key()`, first-read order.
    pub fn read_counts(&self) -> IndexMap<String, usize> {
        self.state().read_counts.clone()
    }

    /// Tag lists passed to `list_items_by_tags`, in order.
    pub fn tag_queries(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                OpCall::ListItemsByTags(tags) => Some(tags),
                _ => None,
            })
            .collect()
    }
}

impl OnePassword for FakeOp {
    fn assert_authenticated(&self) -> Result<()> {
        let mut state = self.state();
        state.calls.push(OpCall::AssertAuthenticated);
        state.auth.clone()
    }

    fn list_items_by_tags(&self, tags: &[String]) -> Result<Vec<ItemSummary>> {
        let (gate, result) = {
            let mut state = self.state();
            state.calls.push(OpCall::ListItemsByTags(tags.to_vec()));
            let result = state
                .list_rules
                .iter()
                .find(|(rule, _)| rule.as_slice() == tags)
                .map(|(_, result)| result.clone())
                .unwrap_or_else(|| state.default_list.clone());
            (state.gate.clone(), result)
        };
        if let Some(gate) = gate
            && !gate.wait()
        {
            return Err(Error::internal(format!(
                "FakeOp({}): concurrency gate timed out; vault scans did not overlap",
                self.vault()
            )));
        }
        result
    }

    fn read_config_item(&self, item: &str) -> Result<String> {
        let mut state = self.state();
        state.calls.push(OpCall::ReadConfigItem(item.to_string()));
        state.config_items.get(item).cloned().unwrap_or_else(|| {
            Err(Error::internal(format!(
                "FakeOp({}): no config item scripted for {item}",
                self.vault()
            )))
        })
    }

    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap> {
        let (hook, result) = {
            let mut state = self.state();
            state.calls.push(OpCall::ReadEnvItem(item.clone()));
            *state.read_counts.entry(item.key().to_string()).or_insert(0) += 1;
            let result = state
                .env_values
                .get(item.key())
                .cloned()
                .unwrap_or_else(|| Ok(EnvMap::new()));
            (state.on_read.clone(), result)
        };
        if let Some(hook) = hook {
            hook(item);
        }
        result
    }
}

struct FactoryState {
    vaults: IndexMap<String, FakeOp>,
    created: Vec<String>,
}

/// [`OpFactory`] handing out registered [`FakeOp`]s by vault. An unknown
/// vault gets a default `FakeOp` that is registered on first use, so tests
/// can inspect it afterwards. Records every `create` call in order.
#[derive(Clone)]
pub struct FakeOpFactory {
    inner: Arc<Mutex<FactoryState>>,
}

impl Default for FakeOpFactory {
    fn default() -> Self {
        FakeOpFactory {
            inner: Arc::new(Mutex::new(FactoryState {
                vaults: IndexMap::new(),
                created: Vec::new(),
            })),
        }
    }
}

impl FakeOpFactory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `op` under `op.vault()`.
    pub fn with(self, op: FakeOp) -> Self {
        self.inner
            .lock()
            .unwrap()
            .vaults
            .insert(op.vault().to_string(), op);
        self
    }

    /// Vault ids passed to `create`, in call order (duplicates kept).
    pub fn created(&self) -> Vec<String> {
        self.inner.lock().unwrap().created.clone()
    }

    /// The fake for `vault` (registering a default one if needed).
    pub fn op(&self, vault: &str) -> FakeOp {
        let mut state = self.inner.lock().unwrap();
        state
            .vaults
            .entry(vault.to_string())
            .or_insert_with(|| FakeOp::new(vault))
            .clone()
    }
}

impl OpFactory for FakeOpFactory {
    fn create(&self, vault: &str) -> Arc<dyn OnePassword> {
        self.inner.lock().unwrap().created.push(vault.to_string());
        Arc::new(self.op(vault))
    }
}

struct ListerState {
    result: Option<Result<Vec<Vault>>>,
    calls: usize,
}

/// [`VaultLister`] returning a scripted result and counting calls.
/// [`FakeVaultLister::forbidden`] panics if called (non-TTY guard tests).
#[derive(Clone)]
pub struct FakeVaultLister {
    inner: Arc<Mutex<ListerState>>,
}

impl FakeVaultLister {
    pub fn new(vaults: Vec<Vault>) -> Self {
        FakeVaultLister {
            inner: Arc::new(Mutex::new(ListerState {
                result: Some(Ok(vaults)),
                calls: 0,
            })),
        }
    }

    pub fn failing(error: Error) -> Self {
        FakeVaultLister {
            inner: Arc::new(Mutex::new(ListerState {
                result: Some(Err(error)),
                calls: 0,
            })),
        }
    }

    pub fn forbidden() -> Self {
        FakeVaultLister {
            inner: Arc::new(Mutex::new(ListerState {
                result: None,
                calls: 0,
            })),
        }
    }

    pub fn calls(&self) -> usize {
        self.inner.lock().unwrap().calls
    }
}

impl VaultLister for FakeVaultLister {
    fn list_vaults(&self) -> Result<Vec<Vault>> {
        let mut state = self.inner.lock().unwrap();
        state.calls += 1;
        match &state.result {
            Some(result) => result.clone(),
            None => panic!("FakeVaultLister::forbidden was called"),
        }
    }
}

/// Builds a `Vault`.
pub fn vault(id: &str, name: &str) -> Vault {
    Vault {
        id: id.to_string(),
        name: name.to_string(),
    }
}
