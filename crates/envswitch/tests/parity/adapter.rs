use super::{
    json::J,
    model,
    ports::{self, Calls, Factory, Installer, Prompt, Runner},
    transport::{Bridge, CwdGuard, UmaskGuard},
};
use crate::support::{ConcurrencyGate, EnvVars, EventLog, FakeInstaller, SharedBuf};
use envswitch::{Error, Result, model::*, op::client::OnePasswordClient, ports::*};
use serde_json::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

pub fn execute(s: &J, bundle: &Value, path: &Path, bridge: &Bridge) -> Result<J> {
    let root = PathBuf::from(bundle["paths"]["ROOT"].as_str().unwrap());
    let app = PathBuf::from(bundle["paths"]["APP"].as_str().unwrap());
    let cwd = PathBuf::from(s.get("cwd").str_or(root.to_str().unwrap()));
    let _cwd = CwdGuard::set(&cwd);
    let _umask = UmaskGuard::set(if s.has("umask") {
        s.get("umask").int() as u32
    } else {
        0o022
    });
    let calls = Arc::new(Mutex::new(Calls::default()));
    let scan_order = if s.get("argv").items().is_empty() && s.get("kind").text() == "cli" {
        if s.get("op").has("vaults") {
            s.get("op")
                .get("vaults")
                .items()
                .iter()
                .map(|v| v.get("id").str_or(v.get("name").text()).to_owned())
                .collect()
        } else {
            vec!["alpha".into()]
        }
    } else {
        Vec::new()
    };
    let runner = Arc::new(Runner {
        responses: bundle["responses"].as_array().unwrap().clone(),
        calls: calls.clone(),
        gate: if s.has("parallelScans") {
            Some(ConcurrencyGate::new(
                s.get("parallelScans").int() as usize,
                Duration::from_secs(10),
            ))
        } else {
            None
        },
        order: (Mutex::new(0), Condvar::new()),
        scan_order,
        mutate: if s.has("mutateOnRead") {
            Some((
                s.get("mutateOnRead").get("item").text().into(),
                s.get("mutateOnRead").get("files").clone(),
            ))
        } else {
            None
        },
        mutated: Mutex::new(false),
        bridge: bridge.clone(),
        bundle: path.into(),
    });
    let out = SharedBuf::new();
    let mut err = SharedBuf::new();
    let events = EventLog::new();
    let mut result = J::null();
    let mut code = 0;
    let raw_config = if s.has("configRaw") {
        s.get("configRaw").text().to_owned()
    } else if s.has("config") {
        s.get("config").json()
    } else {
        model::default_config().json()
    };
    let config = || envswitch::config::parse_config(&raw_config);
    let op = OnePasswordClient::new(s.get("vault").str_or("alpha"), runner.clone());
    let operation = (|| -> Result<()> {
        match s.get("kind").text() {
            "parseArgs" => {
                result = model::args(&envswitch::cli::parse_args(&s.get("argv").strings()))
            }
            "formatProgressEvent" => {
                let e = s.get("event");
                result = J::string(envswitch::cli::format_progress_event(
                    &ProgressEvent::new(
                        model::status(e.get("status").text()),
                        e.get("message").text(),
                    ),
                    s.get("color").boolean(),
                ));
            }
            "config" => result = model::config(&config()?),
            "tags" => {
                let c = config()?;
                let p = s.get("params");
                let param = TagParams {
                    env: if p.has("env") {
                        Some(p.get("env").text())
                    } else {
                        None
                    },
                    client: if p.has("client") {
                        Some(p.get("client").text())
                    } else {
                        None
                    },
                    target: if p.has("target") {
                        Some(p.get("target").text())
                    } else {
                        None
                    },
                    group: if p.has("group") {
                        Some(p.get("group").text())
                    } else {
                        None
                    },
                    layer: model::layer(p.get("layer").text()),
                };
                result = J::object([
                    (
                        "tags",
                        model::tag_values(&envswitch::config::tag_values(&c, &param, &cwd)),
                    ),
                    (
                        "candidates",
                        model::strings(&envswitch::config::candidate_list_tags(
                            &c,
                            p.get("env").str_or("local"),
                            &cwd,
                        )),
                    ),
                ]);
            }
            "path" => {
                result = J::string(
                    envswitch::config::resolve_output_path(
                        Path::new(s.get("root").str_or(root.to_str().unwrap())),
                        s.get("output").text(),
                    )?
                    .to_string_lossy(),
                )
            }
            "classify" => {
                let p = s.get("params");
                result = envswitch::switcher::classify_item_for_target(
                    &config()?,
                    &model::item_input(s.get("item")),
                    &ClassifyParams {
                        env: p.get("env").str_or("local").into(),
                        client: p.get("client").str_or("alpha").into(),
                        target: p.get("target").str_or("api").into(),
                    },
                    &cwd,
                )
                .map(|l| J::string(l.as_str()))
                .unwrap_or(J::null());
            }
            "inspect" => {
                result = model::inspect(&envswitch::switcher::inspect_project(
                    &config()?,
                    Path::new(s.get("root").str_or(root.to_str().unwrap())),
                )?)
            }
            "switch" => {
                let o = s.get("options");
                let req = SwitchRequest {
                    config: config()?,
                    root: PathBuf::from(
                        o.get("root")
                            .str_or(s.get("root").str_or(root.to_str().unwrap())),
                    ),
                    cwd: cwd.clone(),
                    env: o.get("env").str_or("local").into(),
                    client: o.get("client").str_or("alpha").into(),
                    target_name: o.get("targetName").str_or("all").into(),
                    dry_run: o.get("dryRun").boolean(),
                };
                result = model::switch(&envswitch::switcher::switch_environment(
                    &op,
                    &req,
                    &mut |e| {
                        if !s.has("log") || s.get("log").boolean() {
                            events.push(e)
                        }
                    },
                )?);
            }
            "dotenv" => {
                let values = model::env_input(s.get("values"));
                if s.get("write").boolean() {
                    let output = s.get("output").str_or("nested/.env");
                    let output = if Path::new(output).is_absolute() {
                        PathBuf::from(output)
                    } else {
                        root.join(output)
                    };
                    envswitch::dotenv::write_env_file(&output, &values)?
                } else {
                    result = J::string(envswitch::dotenv::stringify_env(&values)?)
                }
            }
            "op" => match s.get("method").text() {
                "auth" => op.assert_authenticated()?,
                "vaults" => {
                    result = J::array(
                        envswitch::op::client::list_vaults(runner.as_ref())?
                            .iter()
                            .map(|v| {
                                J::object([("id", J::string(&v.id)), ("name", J::string(&v.name))])
                            }),
                    )
                }
                "items" => {
                    result = J::array(
                        op.list_items_by_tags(&s.get("tags").strings())?
                            .iter()
                            .map(model::item),
                    )
                }
                "config" => result = J::string(op.read_config_item(s.get("item").str_or("cfg"))?),
                "env" => {
                    let i = if s.has("item") {
                        model::item_input(s.get("item"))
                    } else {
                        ItemSummary {
                            id: "alpha".into(),
                            title: "alpha".into(),
                            tags: vec![],
                        }
                    };
                    result = model::env(&op.read_env_item(&i)?);
                }
                _ => return Err(Error::internal("unknown op method")),
            },
            "cli" => {
                let vars = EnvVars::new();
                for (k, v) in s.get("env").pairs() {
                    if !v.is_null() {
                        vars.set(k, v.text());
                    }
                }
                if let Some(install_dir) = bundle["native"]["installDir"].as_str() {
                    // The synthetic launcher is already on the injected PATH; no profile change is needed.
                    vars.set("PATH", install_dir);
                    let home = bundle["paths"]["HOME"].as_str().unwrap();
                    vars.set("HOME", home).set("USERPROFILE", home);
                }
                let update = s.get("update");
                let native_actions = bundle["native"]["onInstall"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let native_bridge = bridge.clone();
                let native_path = path.to_path_buf();
                let fake = FakeInstaller::new()
                    .with_result(if update.has("installerError") {
                        Err(Error::internal(format!(
                            "update failed: could not run installer: {}",
                            update.get("installerError").text()
                        )))
                    } else {
                        Ok(update.get("exitCode").int() as i32)
                    })
                    .on_run(move |_| {
                        if !native_actions.is_empty() {
                            native_bridge
                                .run(
                                    &["actions".into(), native_path.display().to_string()],
                                    Some(&serde_json::to_string(&native_actions).unwrap()),
                                )
                                .expect("native fixture installer");
                        }
                    });
                let interactive = if s.has("isInteractive") {
                    Some(s.get("isInteractive").boolean())
                } else {
                    None
                };
                let mut deps = Deps {
                    cwd: cwd.clone(),
                    root: PathBuf::from(s.get("root").str_or(root.to_str().unwrap())),
                    env: vars.lookup(),
                    stdin_is_tty: interactive.unwrap_or(s.get("tty").get("stdin").boolean()),
                    stdout_is_tty: interactive.unwrap_or(s.get("tty").get("stdout").boolean()),
                    stderr_is_tty: s.get("tty").get("stderr").boolean(),
                    out: out.boxed(),
                    err: err.boxed(),
                    log: None,
                    op_factory: Box::new(Factory {
                        runner: runner.clone(),
                    }),
                    vault_lister: ports::lister(runner.clone()),
                    prompter: if s.has("prompts") {
                        Some(Box::new(Prompt {
                            answers: Mutex::new(0),
                            scenario: s.clone(),
                            calls: calls.clone(),
                        }))
                    } else {
                        None
                    },
                    current_exe: Some(
                        bundle["native"]["currentExe"]
                            .as_str()
                            .map(PathBuf::from)
                            .unwrap_or_else(|| {
                                app.join("bin").join(format!(
                                    "envswitch_{}_{}{}",
                                    env!("CARGO_PKG_VERSION"),
                                    env!("TARGET"),
                                    std::env::consts::EXE_SUFFIX
                                ))
                            }),
                    ),
                    app_dir: Some(app.clone()),
                    installer: Box::new(Installer {
                        fake,
                        stdout: bundle["native"]["stdout"].as_str().unwrap_or("").into(),
                        calls: calls.clone(),
                    }),
                };
                code = envswitch::cli::run(s.get("argv").strings(), &mut deps);
            }
            _ => return Err(Error::internal("unsupported scenario kind")),
        }
        Ok(())
    })();
    if let Err(e) = operation {
        code = e.exit_code();
        if e != Error::Cancelled {
            writeln!(err, "envswitch: {e}").unwrap();
        }
    }
    let c = calls.lock().unwrap();
    assert!(
        c.answers_used >= s.get("prompts").items().len(),
        "unused scripted answers"
    );
    assert!(c.faults.is_empty(), "harness fault: {:?}", c.faults);
    let progress = if s.get("kind").text() == "cli" {
        ports::progress_from_stderr(&err.bytes())
    } else {
        events.events().iter().map(model::progress).collect()
    };
    Ok(J::object([
        ("exitCode", J::number(code)),
        ("result", result),
        ("stdout", J::array(out.bytes().into_iter().map(J::number))),
        ("stderr", J::array(err.bytes().into_iter().map(J::number))),
        ("prompts", J::array(c.prompt_log.clone())),
        ("progress", J::array(progress)),
        (
            "opCalls",
            J::array(c.argv.iter().map(|a| model::strings(a))),
        ),
        ("readCounts", ports::counts(&c.argv)),
        ("factoryCalls", model::strings(&c.factories)),
        ("updateCalls", J::array(c.updates.clone())),
    ]))
}
