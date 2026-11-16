//! `switch`, `list`, `doctor` (contract 01 §2–3). Owner: C4.

use crate::config::{parse_config, tag_values};
use crate::error::{Result, fail};
use crate::model::{Config, LayerName, ProgressEvent, SwitchRequest, TagParams};
use crate::ports::{Deps, OnePassword};
use crate::switcher::{inspect_project, switch_environment};

use super::args::{FlagValue, ParsedArgs};
use super::output::{log_progress, print_switch_results, write_line};

pub(crate) fn validate_flags(args: &ParsedArgs, allowed: &[&str]) -> Result<()> {
    for (name, value) in &args.flags {
        if !allowed.contains(&name.as_str()) {
            return fail(format!("unknown flag: --{name}"));
        }
        if matches!(name.as_str(), "dry-run" | "help") && *value != FlagValue::True {
            return fail(format!("--{name} does not take a value"));
        }
    }
    Ok(())
}

fn string_flag(args: &ParsedArgs, name: &str, default: &str) -> Result<String> {
    match args.flags.get(name) {
        None => Ok(default.to_string()),
        Some(FlagValue::Value(value)) if !crate::jsutil::js_trim(value).is_empty() => {
            Ok(value.clone())
        }
        _ => fail(format!("--{name} requires a value")),
    }
}

fn require_string_flag(args: &ParsedArgs, name: &str) -> Result<String> {
    let value = string_flag(args, name, "")?;
    if value.is_empty() {
        return fail(format!("--{name} is required"));
    }
    Ok(value)
}

pub(crate) fn load_config(
    op: &dyn OnePassword,
    item: &str,
    log_name: &str,
    deps: &mut Deps,
) -> Result<Config> {
    log_progress(
        deps,
        ProgressEvent::info(format!("reading config item {log_name}")),
    );
    let config = parse_config(&op.read_config_item(item)?)?;
    log_progress(
        deps,
        ProgressEvent::done(format!("loaded config item {log_name}")),
    );
    Ok(config)
}

/// TS `runSwitch` (01 §3.1).
pub(crate) fn run_switch(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    validate_flags(
        args,
        &[
            "vault",
            "config-item",
            "client",
            "env",
            "target",
            "dry-run",
            "help",
        ],
    )?;
    let vault = require_string_flag(args, "vault")?;
    let config_item = require_string_flag(args, "config-item")?;
    let client = require_string_flag(args, "client")?;
    let env = string_flag(args, "env", "local")?;
    let target_name = string_flag(args, "target", "all")?;
    let dry_run = args.flags.get("dry-run") == Some(&FlagValue::True);
    let op = deps.op_factory.create(&vault);
    let config = load_config(op.as_ref(), &config_item, &config_item, deps)?;
    let req = SwitchRequest {
        config,
        root: deps.root.clone(),
        cwd: deps.cwd.clone(),
        env,
        client,
        target_name,
        dry_run,
    };
    let results = switch_environment(op.as_ref(), &req, &mut |event| log_progress(deps, event))?;
    print_switch_results(&results, &req.root, dry_run, deps)
}

/// TS `runList` (01 §3.2): load config before validating the subject.
pub(crate) fn run_list(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    validate_flags(args, &["vault", "config-item", "help"])?;
    let vault = require_string_flag(args, "vault")?;
    let config_item = require_string_flag(args, "config-item")?;
    let subject = args
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or("targets");
    let op = deps.op_factory.create(&vault);
    let config = load_config(op.as_ref(), &config_item, &config_item, deps)?;
    match subject {
        "clients" => {
            let clients = config.clients.as_deref().unwrap_or_default();
            if clients.is_empty() {
                write_line(deps, "(none)")?;
            }
            for client in clients {
                write_line(deps, client)?;
            }
        }
        "targets" => {
            for target in &config.targets {
                write_line(deps, format!("{}\t{}", target.name, target.output))?;
            }
        }
        "layers" => {
            for layer in LayerName::ALL {
                let tags = tag_values(
                    &config,
                    &TagParams {
                        layer: Some(layer),
                        ..TagParams::default()
                    },
                    &deps.cwd,
                );
                write_line(deps, format!("{layer}\t{}", tags.layer.unwrap_or_default()))?;
            }
        }
        _ => return fail(format!("unknown list subject: {subject}")),
    }
    Ok(())
}

/// TS `runDoctor` (01 §3.3).
pub(crate) fn run_doctor(args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    validate_flags(args, &["vault", "config-item", "help"])?;
    let vault = require_string_flag(args, "vault")?;
    let config_item = require_string_flag(args, "config-item")?;
    let op = deps.op_factory.create(&vault);
    log_progress(
        deps,
        ProgressEvent::info("checking 1Password authentication"),
    );
    op.assert_authenticated()?;
    log_progress(deps, ProgressEvent::done("1Password authentication ok"));
    let config = load_config(op.as_ref(), &config_item, &config_item, deps)?;
    log_progress(
        deps,
        ProgressEvent::info(format!("inspecting project root {}", deps.root.display())),
    );
    let report = inspect_project(&config, &deps.root)?;
    write_line(deps, "1Password authentication: ok")?;
    write_line(deps, format!("project root: {}", deps.root.display()))?;
    write_line(deps, format!("targets: {}", report.targets.len()))?;
    for target in report.targets {
        let status = if target.parent_exists {
            "ok"
        } else {
            "missing parent"
        };
        write_line(
            deps,
            format!(
                "- {} -> {} ({status})",
                target.target.name,
                target.output_path.display()
            ),
        )?;
    }
    Ok(())
}
