//! Provisional fakes for the `update` ports (D3 may extend them).

use std::sync::{Arc, Mutex};

use envswitch::error::Result;
use envswitch::ports::{InstallPlan, InstallerRunner};

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
