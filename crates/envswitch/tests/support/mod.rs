//! Shared integration-test support (S1). Include with `mod support;` from any
//! `tests/<area>_*.rs` file.
//!
//! * [`FakeOp`], [`FakeOpFactory`], [`FakeVaultLister`], [`ConcurrencyGate`]:
//!   thread-safe 1Password fakes recording calls, argv, and read counts.
//! * [`ScriptedPrompter`]: records prompts, replays [`Answer`]s.
//! * [`SharedBuf`], [`EventLog`], [`EnvVars`]: captured streams, progress
//!   events, and a fake environment.
//! * [`TempProject`]: temp project roots with `package.json` markers, files,
//!   and directory links (symlink / junction).
//! * [`FakeReleaseSource`], [`FakeInstaller`]: provisional `update` fakes.
//! * [`deps`]: a fully faked `Deps` plus handles to every fake.
//!
//! Every fake is `Clone` and shares state through `Arc<Mutex<_>>`, so a test
//! keeps a handle after moving a boxed clone into `Deps`.

// Not every test file uses every helper.
#![allow(dead_code, unused_imports)]

pub mod capture;
pub mod fake_op;
pub mod project;
pub mod prompter;
pub mod update;

use std::path::Path;

use envswitch::ports::Deps;

pub use capture::{EnvVars, EventLog, SharedBuf};
pub use fake_op::{
    ConcurrencyGate, FakeOp, FakeOpFactory, FakeVaultLister, OpCall, env_map, item, vault,
};
pub use project::{TempProject, link_dir};
pub use prompter::{Answer, PromptRecord, ScriptedPrompter};
pub use update::{FakeInstaller, FakeReleaseSource};

/// A faked [`Deps`] and handles to each fake inside it.
pub struct TestDeps {
    pub deps: Deps,
    pub out: SharedBuf,
    pub err: SharedBuf,
    /// Receives progress events while `deps.log` is the default sink.
    pub events: EventLog,
    pub env: EnvVars,
    pub factory: FakeOpFactory,
    pub vaults: FakeVaultLister,
    pub release: FakeReleaseSource,
    pub installer: FakeInstaller,
}

/// Fully faked `Deps` for `root`:
///
/// * `cwd = root = root`; all three TTY flags `false`; empty environment;
/// * `out`/`err` captured; progress goes to [`TestDeps::events`] (set
///   `deps.log = None` to test formatted stderr progress instead);
/// * empty [`FakeOpFactory`] and [`FakeVaultLister`] (no vaults);
/// * no prompter (so guided mode hits the non-TTY guard unless a test sets
///   one with [`TestDeps::with_prompter`]);
/// * `current_exe`/`app_dir` `None`; release source returns `None`;
///   installer exits 0.
///
/// Nothing here can reach a real `op`, network, or installer.
pub fn deps(root: &Path) -> TestDeps {
    let out = SharedBuf::new();
    let err = SharedBuf::new();
    let events = EventLog::new();
    let env = EnvVars::new();
    let factory = FakeOpFactory::new();
    let vaults = FakeVaultLister::new(Vec::new());
    let release = FakeReleaseSource::new(None);
    let installer = FakeInstaller::new();
    let deps = Deps {
        cwd: root.to_path_buf(),
        root: root.to_path_buf(),
        env: env.lookup(),
        stdin_is_tty: false,
        stdout_is_tty: false,
        stderr_is_tty: false,
        out: out.boxed(),
        err: err.boxed(),
        log: Some(events.sink()),
        op_factory: Box::new(factory.clone()),
        vault_lister: Box::new(vaults.clone()),
        prompter: None,
        current_exe: None,
        app_dir: None,
        release_source: Box::new(release.clone()),
        installer: Box::new(installer.clone()),
    };
    TestDeps {
        deps,
        out,
        err,
        events,
        env,
        factory,
        vaults,
        release,
        installer,
    }
}

impl TestDeps {
    /// Replaces the op factory (and its handle).
    pub fn with_factory(mut self, factory: FakeOpFactory) -> Self {
        self.deps.op_factory = Box::new(factory.clone());
        self.factory = factory;
        self
    }

    /// Replaces the vault lister (and its handle).
    pub fn with_vaults(mut self, vaults: FakeVaultLister) -> Self {
        self.deps.vault_lister = Box::new(vaults.clone());
        self.vaults = vaults;
        self
    }

    /// Injects a prompter (bypasses the TTY guard, like TS `prompt`).
    pub fn with_prompter(mut self, prompter: &ScriptedPrompter) -> Self {
        self.deps.prompter = Some(prompter.boxed());
        self
    }

    pub fn with_release(mut self, release: FakeReleaseSource) -> Self {
        self.deps.release_source = Box::new(release.clone());
        self.release = release;
        self
    }

    pub fn with_installer(mut self, installer: FakeInstaller) -> Self {
        self.deps.installer = Box::new(installer.clone());
        self.installer = installer;
        self
    }

    /// Sends progress to formatted stderr instead of [`TestDeps::events`].
    pub fn without_event_capture(mut self) -> Self {
        self.deps.log = None;
        self
    }

    /// `envswitch::run(argv, &mut deps)`.
    pub fn run(&mut self, argv: &[&str]) -> i32 {
        let argv = argv.iter().map(|arg| arg.to_string()).collect();
        envswitch::run(argv, &mut self.deps)
    }
}
