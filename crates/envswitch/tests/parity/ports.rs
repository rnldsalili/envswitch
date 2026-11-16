use super::{json::J, model, transport::Bridge};
use crate::support::{Answer, ConcurrencyGate, FakeInstaller, FakeReleaseSource, ScriptedPrompter};
use envswitch::op::client::{OnePasswordClient, OpVaultLister};
use envswitch::{Error, Result, model::*, ports::*};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

#[derive(Default)]
pub struct Calls {
    pub argv: Vec<Vec<String>>,
    pub factories: Vec<String>,
    pub updates: Vec<J>,
    pub prompt_log: Vec<J>,
    pub faults: Vec<String>,
    pub answers_used: usize,
}
pub struct Runner {
    pub responses: Vec<Value>,
    pub calls: Arc<Mutex<Calls>>,
    pub gate: Option<Arc<ConcurrencyGate>>,
    pub order: (Mutex<usize>, Condvar),
    pub scan_order: Vec<String>,
    pub mutate: Option<(String, J)>,
    pub mutated: Mutex<bool>,
    pub bridge: Bridge,
    pub bundle: PathBuf,
}
impl CommandRunner for Runner {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        assert_eq!(program, "op");
        let scan = args.first().map(String::as_str) == Some("item")
            && args.get(1).map(String::as_str) == Some("list")
            && args.last().map(String::as_str) == Some("envswitch-config");
        if scan && !self.scan_order.is_empty() {
            let vault = args.get(3).unwrap();
            let (lock, cv) = &self.order;
            let count = lock.lock().unwrap();
            let (mut count, timeout) = cv
                .wait_timeout_while(count, Duration::from_secs(10), |n| {
                    self.scan_order.get(*n) != Some(vault)
                })
                .unwrap();
            if timeout.timed_out() {
                self.calls
                    .lock()
                    .unwrap()
                    .faults
                    .push("config scan scheduling timed out".into());
                return Err(Error::internal("config scan scheduling timed out"));
            }
            self.calls.lock().unwrap().argv.push(args.to_vec());
            *count += 1;
            cv.notify_all();
        } else {
            self.calls.lock().unwrap().argv.push(args.to_vec());
        }
        if scan
            && let Some(gate) = &self.gate
            && !gate.wait()
        {
            self.calls
                .lock()
                .unwrap()
                .faults
                .push("parallel scan barrier timed out".into());
            return Err(Error::internal("parallel scan barrier timed out"));
        }
        if let Some((item, actions)) = &self.mutate
            && args.get(1).map(String::as_str) == Some("get")
            && args.get(2) == Some(item)
        {
            let mut done = self.mutated.lock().unwrap();
            if !*done {
                *done = true;
                self.bridge.actions(&self.bundle, actions);
            }
        }
        let response = self
            .responses
            .iter()
            .find(|r| r["argv"] == serde_json::json!(args));
        let Some(r) = response else {
            let message = format!("unconfigured fake-op argv: {args:?}");
            self.calls.lock().unwrap().faults.push(message.clone());
            return Err(Error::internal(message));
        };
        let stdout = if let Some(v) = r["stdoutBase64"].as_str() {
            self.bridge.decode_base64(v)
        } else if let Some(n) = r["stdoutSize"]["size"].as_u64() {
            vec![r["stdoutSize"]["byte"].as_u64().unwrap() as u8; n as usize]
        } else {
            r["stdout"].as_str().unwrap_or("").as_bytes().to_vec()
        };
        Ok(CommandOutput {
            code: Some(r["exitCode"].as_i64().unwrap_or(0) as i32),
            stdout,
            stderr: r["stderr"].as_str().unwrap_or("").as_bytes().to_vec(),
        })
    }
}
pub struct Factory {
    pub runner: Arc<Runner>,
}
impl OpFactory for Factory {
    fn create(&self, vault: &str) -> Arc<dyn OnePassword> {
        self.runner
            .calls
            .lock()
            .unwrap()
            .factories
            .push(vault.into());
        Arc::new(OnePasswordClient::new(vault, self.runner.clone()))
    }
}
pub fn lister(r: Arc<Runner>) -> Box<dyn VaultLister> {
    Box::new(OpVaultLister::new(r))
}

pub struct Prompt {
    pub answers: Mutex<usize>,
    pub scenario: J,
    pub calls: Arc<Mutex<Calls>>,
}
impl Prompt {
    fn answer(&self, message: &str) -> J {
        let mut i = self.answers.lock().unwrap();
        let answer = self.scenario.get("prompts").at(*i).clone();
        *i += 1;
        self.calls.lock().unwrap().answers_used = *i;
        if !answer.is_null() {
            return answer;
        }
        let _ = message;
        J::null()
    }
    fn spec(&self, spec: &SelectSpec) -> Result<Vec<J>> {
        let s = &self.scenario;
        let mut values: Vec<(String, J)> = Vec::new();
        match spec.message.as_str() {
            "Mode" => values.extend([
                ("Dry run".into(), J::bool(true)),
                ("Write .env files".into(), J::bool(false)),
            ]),
            "Vault" => {
                let vaults = if s.get("op").has("vaults") {
                    s.get("op").get("vaults").clone()
                } else {
                    J::array([J::object([
                        ("id", J::string("alpha")),
                        ("name", J::string("alpha")),
                    ])])
                };
                for v in vaults.items() {
                    let id = v.get("id").text();
                    let name = v.get("name").str_or(id);
                    let label = if id.is_empty() || id == name {
                        name.to_owned()
                    } else {
                        format!("{name} ({id})")
                    };
                    values.push((label, J::string(if id.is_empty() { name } else { id })));
                }
            }
            "Config item" => {
                let default = J::array([J::object([
                    ("id", J::string("cfg")),
                    ("title", J::string("demo-suite")),
                ])]);
                let mut lists = vec![s.get("op").get("configItems").clone(), default];
                lists.extend(
                    s.get("op")
                        .get("byVault")
                        .pairs()
                        .iter()
                        .map(|(_, v)| v.get("configItems").clone()),
                );
                for list in lists {
                    for item in list.items() {
                        let id = item.get("id").text();
                        let title = if item.get("title").text().is_empty() {
                            id
                        } else {
                            item.get("title").text()
                        };
                        let label = if id.is_empty() || id == title {
                            title.to_owned()
                        } else {
                            format!("{title} ({id})")
                        };
                        values.push((label, J::string(if id.is_empty() { title } else { id })));
                    }
                }
            }
            "Client" | "Environment" => {
                let key = if spec.message == "Client" {
                    "clients"
                } else {
                    "environments"
                };
                values.extend(
                    s.get("config")
                        .get(key)
                        .items()
                        .iter()
                        .map(|v| (v.text().into(), v.clone())),
                );
            }
            "Target" => {
                values.push(("all".into(), J::string("all")));
                values.extend(
                    s.get("config")
                        .get("targets")
                        .items()
                        .iter()
                        .map(|v| (v.get("name").text().into(), v.get("name").clone())),
                );
            }
            _ => return Err(self.fault("unknown prompt message")),
        }
        spec.choices
            .iter()
            .map(|choice| {
                values
                    .iter()
                    .find(|(label, _)| *label == choice.label)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| {
                        self.fault(&format!(
                            "unrecognized prompt choice: {}: {}",
                            spec.message, choice.label
                        ))
                    })
            })
            .collect()
    }
    fn special(a: &J) -> Option<Answer> {
        if a.get("cancel").boolean() {
            Some(Answer::Cancel)
        } else if a.has("error") {
            Some(if a.get("error").get("name").text() == "ExitPromptError" {
                Answer::Cancel
            } else {
                Answer::Fail(Error::internal(a.get("error").get("message").text()))
            })
        } else {
            None
        }
    }
    fn fault(&self, message: &str) -> Error {
        self.calls.lock().unwrap().faults.push(message.into());
        Error::internal(message)
    }
}
impl Prompter for Prompt {
    fn select(&self, spec: &SelectSpec) -> Result<usize> {
        let values = self.spec(spec)?;
        self.calls.lock().unwrap().prompt_log.push(J::object([
            ("kind", J::string("select")),
            ("message", J::string(&spec.message)),
            (
                "choices",
                J::array(spec.choices.iter().zip(&values).map(|(c, v)| {
                    J::object([("label", J::string(&c.label)), ("value", v.clone())])
                })),
            ),
            (
                "default",
                spec.default_index
                    .map(|i| values[i].clone())
                    .unwrap_or(J::null()),
            ),
            ("required", J::null()),
        ]));
        let answer = self.answer(&spec.message);
        let scripted = if let Some(a) = Self::special(&answer) {
            a
        } else if answer.is_null() || answer.get("default").boolean() {
            Answer::Default
        } else if answer.has("index") {
            Answer::Index(answer.get("index").int() as usize)
        } else {
            let index = values
                .iter()
                .position(|v| *v == answer)
                .ok_or_else(|| self.fault("invalid scripted select answer"))?;
            Answer::Index(index)
        };
        ScriptedPrompter::new([scripted]).select(spec)
    }
    fn input(&self, spec: &InputSpec) -> Result<String> {
        self.calls.lock().unwrap().prompt_log.push(J::object([
            ("kind", J::string("input")),
            ("message", J::string(&spec.message)),
            ("choices", J::null()),
            (
                "default",
                spec.default.as_ref().map(J::string).unwrap_or(J::null()),
            ),
            ("required", J::bool(spec.required)),
        ]));
        let answer = self.answer(&spec.message);
        let scripted = if let Some(a) = Self::special(&answer) {
            a
        } else if answer.is_null() || answer.get("default").boolean() {
            Answer::Default
        } else {
            Answer::Text(answer.text().into())
        };
        let value = ScriptedPrompter::new([scripted]).input(spec)?;
        if spec.required && value.is_empty() {
            return Err(self.fault("empty required input"));
        }
        Ok(value)
    }
}
pub struct Release {
    pub fake: FakeReleaseSource,
    pub calls: Arc<Mutex<Calls>>,
}
impl ReleaseSource for Release {
    fn latest_version(&self) -> Option<String> {
        self.calls
            .lock()
            .unwrap()
            .updates
            .push(J::object([("method", J::string("fetchLatestVersion"))]));
        self.fake.latest_version()
    }
}
pub struct Installer {
    pub fake: FakeInstaller,
    pub calls: Arc<Mutex<Calls>>,
}
impl InstallerRunner for Installer {
    fn run(&self, plan: &InstallPlan) -> Result<i32> {
        self.calls.lock().unwrap().updates.push(J::object([
            ("method", J::string("runInstaller")),
            ("source", J::string("embedded")),
            (
                "plan",
                J::object([
                    ("repo", J::string(&plan.repo)),
                    ("appRoot", J::string(plan.app_root.to_string_lossy())),
                    (
                        "installDir",
                        plan.install_dir
                            .as_ref()
                            .map(|p| J::string(p.to_string_lossy()))
                            .unwrap_or(J::null()),
                    ),
                    (
                        "version",
                        plan.version.as_ref().map(J::string).unwrap_or(J::null()),
                    ),
                    ("currentVersion", J::string(&plan.current_version)),
                ]),
            ),
        ]));
        self.fake.run(plan)
    }
}
pub fn counts(calls: &[Vec<String>]) -> J {
    let mut counts = J::object(Vec::<(String, J)>::new());
    for a in calls {
        if a.first().map(String::as_str) == Some("item")
            && a.get(1).map(String::as_str) == Some("get")
        {
            let i = a.iter().position(|v| v == "--vault").unwrap();
            let key = format!("{}/{}", a[i + 1], a[2]);
            let n = counts.get(&key).int() + 1;
            counts.set(&key, J::number(n))
        }
    }
    counts
}
pub fn progress_from_stderr(bytes: &[u8]) -> Vec<J> {
    let ansi = regex::Regex::new("\\x1b\\[[0-9;]*m").unwrap();
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let line = ansi.replace_all(line, "");
            let rest = line.strip_prefix("envswitch [")?;
            let (status, message) = rest.split_once("] ")?;
            Some(model::progress(&ProgressEvent::new(
                model::status(status),
                message,
            )))
        })
        .collect()
}
