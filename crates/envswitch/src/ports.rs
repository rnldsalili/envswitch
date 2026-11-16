//! Ports (traits) and the injected dependency bundle [`Deps`] (PLAN §2.3).
//! Frozen by S1; changes need orchestrator approval.
//!
//! # Interface summary
//!
//! | Port                | Real impl (owner)                       | Test fake (`tests/support`) |
//! |---------------------|-----------------------------------------|-----------------------------|
//! | [`CommandRunner`]   | `op::runner::ExecCommandRunner` (C2)    | (C2 writes its own static runner) |
//! | [`OnePassword`]     | `op::client::OnePasswordClient` (C2)    | `FakeOp`                    |
//! | [`OpFactory`]       | `op::client::OpClientFactory` (C2)      | `FakeOpFactory`             |
//! | [`VaultLister`]     | `op::client::OpVaultLister` (C2)        | `FakeVaultLister`           |
//! | [`Prompter`]        | `cli::guided::InquirePrompter` (C4)     | `ScriptedPrompter`          |
//! | [`InstallerRunner`] | `install::EmbeddedInstaller` (D3)       | `FakeInstaller`             |
//!
//! # Thread rules
//!
//! * `OnePassword`, `OpFactory`, `VaultLister`, `CommandRunner` and
//!   `InstallerRunner` are `Send + Sync`.
//! * The guided vault scan (contract 01 §5.2 step 2) creates one client per
//!   vault **on the main thread, in `list_vaults` order** (so factory calls are
//!   deterministic), then runs each `list_items_by_tags(["envswitch-config"])`
//!   on its own `std::thread::scope` thread. Workers return their result
//!   through the scoped `JoinHandle` (or an `mpsc` channel); they never log.
//! * The main thread joins in vault order and only then emits progress
//!   (`skipping vault ...`, `found N vault(s) with config items`) in vault
//!   order. Loggers (`FnMut(ProgressEvent)`) and `Deps::out`/`err` never cross
//!   threads.
//! * [`Prompter`] is main-thread only and is not `Send`.
//! * Everything else in the program is sequential (`op` reads included).
//!
//! # Prompt schema
//!
//! `select(&SelectSpec { message, choices: [Choice { label }], default_index })`
//! returns the chosen index. The boolean Mode prompt is a two-choice select
//! (`["Dry run", "Write .env files"]`, default index 1) mapped back by index.
//! `input(&InputSpec { message, default, required })` returns the raw,
//! untrimmed answer. Ctrl-C and Esc return `Err(Error::Cancelled)` (exit 130).
//! "Auto-select when exactly one choice" is CLI logic: the prompter is not
//! called at all in that case.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::model::{EnvMap, ItemSummary, ProgressEvent, Vault};

// ---------------------------------------------------------------------------
// Process execution
// ---------------------------------------------------------------------------

/// Captured result of a finished child process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    /// Exit code; `None` when the process was killed by a signal.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    /// `code == Some(0)`.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs a program without a shell (contract 02 §5.1). Implementations return
/// `Ok` for any process that ran to completion, whatever its exit code, and
/// `Err(Error::Internal)` when it could not be spawned or exceeded the stdout
/// cap. Turning a non-zero exit into the TS error message is the caller's job
/// (`op::client`), so fakes stay trivial.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput>;
}

// ---------------------------------------------------------------------------
// 1Password
// ---------------------------------------------------------------------------

/// A 1Password client bound to one vault (TS `OnePasswordClient` instance
/// methods; contract 02 §5.2–5.4). Every call is one `op` invocation.
pub trait OnePassword: Send + Sync {
    /// `op whoami --format json`; stdout ignored.
    fn assert_authenticated(&self) -> Result<()>;

    /// `op item list --vault <v> --format json [--tags a,b]` (`--tags` only
    /// when `tags` is non-empty). API order is preserved.
    fn list_items_by_tags(&self, tags: &[String]) -> Result<Vec<ItemSummary>>;

    /// `op item get <item> --vault <v> --format json --reveal`; value of the
    /// first field labeled exactly `config`, untrimmed.
    fn read_config_item(&self, item: &str) -> Result<String>;

    /// `op item get <item.key()> ...`; env values from field labels in field
    /// order (last duplicate wins, keeps first position).
    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap>;
}

/// `createOnePasswordClient(vault)`: no I/O, infallible.
pub trait OpFactory: Send + Sync {
    fn create(&self, vault: &str) -> Arc<dyn OnePassword>;
}

/// `OnePasswordClient.listVaults()`: `op vault list --format json`.
pub trait VaultLister: Send + Sync {
    fn list_vaults(&self) -> Result<Vec<Vault>>;
}

// ---------------------------------------------------------------------------
// Prompts (main thread only)
// ---------------------------------------------------------------------------

/// One select choice. Values are kept by the CLI and mapped back by index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
}

/// `select({ message, choices, defaultValue })`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectSpec {
    pub message: String,
    pub choices: Vec<Choice>,
    /// Index of `defaultValue` in `choices`; `None` when TS passes no default.
    pub default_index: Option<usize>,
}

/// `input({ message, defaultValue, required })`. Every TS call site passes
/// `required: true`; the field is kept so the prompt log matches the oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSpec {
    pub message: String,
    pub default: Option<String>,
    pub required: bool,
}

/// Interactive prompts (TS `GuidedPrompt`). Main-thread only.
pub trait Prompter {
    /// Returns the chosen index into `spec.choices`.
    /// `Err(Error::Cancelled)` on Ctrl-C or Esc.
    fn select(&self, spec: &SelectSpec) -> Result<usize>;

    /// Returns the raw answer (not trimmed; the default when the user just
    /// presses Enter). `Err(Error::Cancelled)` on Ctrl-C or Esc.
    fn input(&self, spec: &InputSpec) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Update (INSTALLER-PROTOCOL §7.1)
// ---------------------------------------------------------------------------

/// Settings handed to the embedded installer by `envswitch update`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallPlan {
    /// GitHub `owner/name`, e.g. `rnldsalili/envswitch`.
    pub repo: String,
    /// Install root (TS `APP_ROOT`, parent of `current`), from the receipt.
    pub app_root: PathBuf,
    /// Launcher directory from the receipt (DIVERGENCES e3).
    pub install_dir: Option<PathBuf>,
    /// Release tag to pin; `None` = let the installer resolve latest.
    pub version: Option<String>,
    /// Canonical compiled tag (`v<CARGO_PKG_VERSION>`, e1); empty omits it.
    pub current_version: String,
    /// Explicit internal request; ordinary update leaves this false.
    pub verify_attestations: bool,
}

/// Runs the installer with forwarded stdio and returns its
/// exit code (`128 + signal` when killed by a signal). Spawn failure is
/// `Err(Error::Internal)`.
pub trait InstallerRunner: Send + Sync {
    fn run(&self, plan: &InstallPlan) -> Result<i32>;

    /// Forward stdout and retain it for protocol terminal-line validation.
    /// Windows uses the raw process stdout handle to preserve code-page bytes.
    /// Legacy runners supply no outcome and cannot claim success.
    fn run_with_output(&self, plan: &InstallPlan, _out: &mut dyn Write) -> Result<(i32, String)> {
        self.run(plan).map(|code| (code, String::new()))
    }
}

// ---------------------------------------------------------------------------
// Deps
// ---------------------------------------------------------------------------

/// Environment variable lookup (`std::env::var_os` in production).
pub type EnvLookup = Box<dyn Fn(&str) -> Option<OsString>>;

/// Everything `cli::run` touches outside pure computation (TS `CliDeps`).
/// There is no clock abstraction.
pub struct Deps {
    /// Process cwd stand-in, used only for the `{project}` fallback
    /// (`basename(process.cwd())` in TS).
    pub cwd: PathBuf,
    /// Project root (TS `options.root ?? process.cwd()`); not canonicalized.
    pub root: PathBuf,
    /// Environment lookup (`CI`, `NO_COLOR`, `GITHUB_TOKEN`, ...).
    pub env: EnvLookup,
    /// Guided mode requires `stdin_is_tty && stdout_is_tty` unless a
    /// `prompter` is injected.
    pub stdin_is_tty: bool,
    pub stdout_is_tty: bool,
    /// Progress color: `stderr_is_tty && CI unset-or-empty && NO_COLOR unset`.
    pub stderr_is_tty: bool,
    /// stdout (help, summaries, list, doctor, update result lines).
    pub out: Box<dyn Write>,
    /// stderr (formatted progress when `log` is `None`, `envswitch: <msg>`).
    pub err: Box<dyn Write>,
    /// Progress sink override (TS `MainOptions.log`). `None` = format each
    /// event with `cli::format_progress_event` and write it to `err`.
    pub log: Option<Box<dyn FnMut(ProgressEvent)>>,
    pub op_factory: Box<dyn OpFactory>,
    pub vault_lister: Box<dyn VaultLister>,
    /// Injected prompter (TS `MainOptions.prompt`). `Some` bypasses the TTY
    /// guard. `None` = the CLI creates `cli::guided::InquirePrompter` after
    /// the TTY guard passes.
    pub prompter: Option<Box<dyn Prompter>>,
    /// `std::env::current_exe()`; `None` if unavailable. Used by `update`.
    pub current_exe: Option<PathBuf>,
    /// Install-root override for `update` tests; `None` = derive from
    /// `current_exe` + receipt (D3).
    pub app_dir: Option<PathBuf>,
    pub installer: Box<dyn InstallerRunner>,
}

impl Deps {
    /// Production wiring used by `main.rs`. Fails only when the process cwd
    /// cannot be read.
    pub fn real() -> Result<Deps> {
        let cwd = std::env::current_dir()
            .map_err(|err| Error::internal(format!("failed to read current directory: {err}")))?;
        let runner: Arc<dyn CommandRunner> = Arc::new(crate::op::runner::ExecCommandRunner);
        Ok(Deps {
            root: cwd.clone(),
            cwd,
            env: Box::new(|name| std::env::var_os(name)),
            stdin_is_tty: io::stdin().is_terminal(),
            stdout_is_tty: io::stdout().is_terminal(),
            stderr_is_tty: io::stderr().is_terminal(),
            out: Box::new(io::stdout()),
            err: Box::new(io::stderr()),
            log: None,
            op_factory: Box::new(crate::op::client::OpClientFactory::new(Arc::clone(&runner))),
            vault_lister: Box::new(crate::op::client::OpVaultLister::new(runner)),
            prompter: None,
            current_exe: std::env::current_exe().ok(),
            app_dir: None,
            installer: Box::new(crate::install::EmbeddedInstaller),
        })
    }
}
