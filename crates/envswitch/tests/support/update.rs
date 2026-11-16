//! Provisional fakes for the `update` ports (D3 may extend them).

use std::sync::{Arc, Mutex};

use envswitch::error::Result;
use envswitch::ports::{InstallPlan, InstallerRunner, ReleaseSource};

/// [`ReleaseSource`] returning a fixed latest version and counting calls.
#[derive(Clone, Default)]
pub struct FakeReleaseSource {
    inner: Arc<Mutex<(Option<String>, usize)>>,
}

impl FakeReleaseSource {
    pub fn new(latest: Option<&str>) -> Self {
        FakeReleaseSource {
            inner: Arc::new(Mutex::new((latest.map(str::to_string), 0))),
        }
    }

    pub fn calls(&self) -> usize {
        self.inner.lock().unwrap().1
    }
}

impl ReleaseSource for FakeReleaseSource {
    fn latest_version(&self) -> Option<String> {
        let mut state = self.inner.lock().unwrap();
        state.1 += 1;
        state.0.clone()
    }
}

type InstallHook = Arc<dyn Fn(&InstallPlan) + Send + Sync>;

struct InstallerState {
    plans: Vec<InstallPlan>,
    result: Result<i32>,
    hook: Option<InstallHook>,
}

/// [`InstallerRunner`] recording every plan and returning a scripted exit
/// code (default 0). An optional hook runs first (e.g. to write files).
#[derive(Clone)]
pub struct FakeInstaller {
    inner: Arc<Mutex<InstallerState>>,
}

impl Default for FakeInstaller {
    fn default() -> Self {
        FakeInstaller {
            inner: Arc::new(Mutex::new(InstallerState {
                plans: Vec::new(),
                result: Ok(0),
                hook: None,
            })),
        }
    }
}

impl FakeInstaller {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_result(self, result: Result<i32>) -> Self {
        self.inner.lock().unwrap().result = result;
        self
    }

    pub fn on_run(self, hook: impl Fn(&InstallPlan) + Send + Sync + 'static) -> Self {
        self.inner.lock().unwrap().hook = Some(Arc::new(hook));
        self
    }

    pub fn plans(&self) -> Vec<InstallPlan> {
        self.inner.lock().unwrap().plans.clone()
    }
}

impl InstallerRunner for FakeInstaller {
    fn run(&self, plan: &InstallPlan) -> Result<i32> {
        let (hook, result) = {
            let mut state = self.inner.lock().unwrap();
            state.plans.push(plan.clone());
            (state.hook.clone(), state.result.clone())
        };
        if let Some(hook) = hook {
            hook(plan);
        }
        result
    }
}
