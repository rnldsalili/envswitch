//! Guided mode (contract 01 §5) and the real prompter. Owner: C4.

use std::sync::Arc;

use inquire::{InquireError, Select, Text};

use crate::error::{Error, Result, fail};
use crate::model::{Config, ItemSummary, ProgressEvent, SwitchRequest, Vault};
use crate::ports::{Choice, Deps, InputSpec, OnePassword, Prompter, SelectSpec};
use crate::switcher::switch_environment;

use super::commands::load_config;
use super::output::{log_progress, print_switch_results, write_line};

/// Real [`Prompter`] backed by `inquire` (stderr, substring filter, page size
/// 7; Esc and Ctrl-C return `Error::Cancelled`, DIVERGENCES c1–c3).
#[derive(Debug, Clone, Copy, Default)]
pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    fn select(&self, spec: &SelectSpec) -> Result<usize> {
        // shortcut: inquire 0.9.5 cannot disable wrapping, resolve at the prompt UX merge gate.
        Select::new(
            &spec.message,
            spec.choices
                .iter()
                .map(|choice| choice.label.as_str())
                .collect(),
        )
        .with_page_size(7)
        .with_starting_cursor(spec.default_index.unwrap_or(0))
        .raw_prompt()
        .map(|answer| answer.index)
        .map_err(prompt_error)
    }

    fn input(&self, spec: &InputSpec) -> Result<String> {
        let mut prompt = Text::new(&spec.message);
        if let Some(default) = &spec.default {
            prompt = prompt.with_default(default);
        }
        if spec.required {
            prompt = prompt.with_validator(inquire::required!());
        }
        prompt.prompt().map_err(prompt_error)
    }
}

fn prompt_error(error: InquireError) -> Error {
    match error {
        InquireError::OperationCanceled | InquireError::OperationInterrupted => Error::Cancelled,
        other => Error::internal(other.to_string()),
    }
}

struct ConfigVault {
    vault: Vault,
    id: String,
    op: Arc<dyn OnePassword>,
    items: Vec<ItemSummary>,
}

fn select_or_auto(
    prompt: &dyn Prompter,
    message: &str,
    labels: Vec<String>,
    default_index: Option<usize>,
) -> Result<usize> {
    if labels.len() == 1 {
        return Ok(0);
    }
    let spec = SelectSpec {
        message: message.to_string(),
        choices: labels.into_iter().map(|label| Choice { label }).collect(),
        default_index,
    };
    let index = prompt.select(&spec)?;
    if index >= spec.choices.len() {
        return Err(Error::internal(format!(
            "{message}: selected index out of range"
        )));
    }
    Ok(index)
}

fn ask_choice_or_value(
    prompt: &dyn Prompter,
    message: &str,
    choices: &[String],
    default: Option<&str>,
) -> Result<String> {
    if choices.is_empty() {
        return prompt.input(&InputSpec {
            message: message.to_string(),
            default: default.map(str::to_string),
            required: true,
        });
    }
    let default_index = default.and_then(|value| choices.iter().position(|choice| choice == value));
    let index = select_or_auto(prompt, message, choices.to_vec(), default_index)?;
    Ok(choices[index].clone())
}

fn ask_vault(prompt: &dyn Prompter, deps: &mut Deps) -> Result<ConfigVault> {
    log_progress(deps, ProgressEvent::info("listing 1Password vaults"));
    let vaults = deps.vault_lister.list_vaults()?;
    log_progress(
        deps,
        ProgressEvent::done(format!("found {} vault(s)", vaults.len())),
    );
    if vaults.is_empty() {
        return fail("no 1Password vaults found");
    }
    log_progress(
        deps,
        ProgressEvent::info("checking vaults for config items tagged envswitch-config"),
    );
    let clients: Vec<_> = vaults
        .into_iter()
        .map(|vault| {
            let id = if vault.id.is_empty() {
                vault.name.clone()
            } else {
                vault.id.clone()
            };
            let op = deps.op_factory.create(&id);
            (vault, id, op)
        })
        .collect();
    let scans = std::thread::scope(|scope| {
        let handles: Vec<_> = clients
            .iter()
            .map(|(_, _, op)| {
                scope.spawn(move || op.list_items_by_tags(&["envswitch-config".to_string()]))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(Error::internal("vault scan thread panicked")))
            })
            .collect::<Vec<_>>()
    });
    let mut choices = Vec::new();
    for ((vault, id, op), scan) in clients.into_iter().zip(scans) {
        match scan {
            Ok(items) if !items.is_empty() => choices.push(ConfigVault {
                vault,
                id,
                op,
                items,
            }),
            Ok(_) => {}
            Err(_) => log_progress(
                deps,
                ProgressEvent::info(format!(
                    "skipping vault {} (could not list items)",
                    vault.name
                )),
            ),
        }
    }
    log_progress(
        deps,
        ProgressEvent::done(format!(
            "found {} vault(s) with config items",
            choices.len()
        )),
    );
    if choices.is_empty() {
        return fail("no 1Password vaults contain items tagged envswitch-config");
    }
    let index = select_or_auto(
        prompt,
        "Vault",
        choices
            .iter()
            .map(|choice| {
                let vault = &choice.vault;
                if vault.id.is_empty() || vault.id == vault.name {
                    vault.name.clone()
                } else {
                    format!("{} ({})", vault.name, vault.id)
                }
            })
            .collect(),
        None,
    )?;
    let index = choices
        .iter()
        .position(|choice| choice.id == choices[index].id)
        .unwrap();
    Ok(choices.remove(index))
}

fn ask_config_item<'a>(prompt: &dyn Prompter, items: &'a [ItemSummary]) -> Result<&'a ItemSummary> {
    if items.is_empty() {
        return fail("no config items tagged envswitch-config found in the selected vault");
    }
    let index = select_or_auto(
        prompt,
        "Config item",
        items
            .iter()
            .map(|item| {
                if item.id.is_empty() || item.id == item.title {
                    item.display_name().to_string()
                } else {
                    format!("{} ({})", item.display_name(), item.id)
                }
            })
            .collect(),
        None,
    )?;
    Ok(items
        .iter()
        .find(|item| item.key() == items[index].key())
        .unwrap())
}

fn default_environment(config: &Config) -> &str {
    let environments = config.environments.as_deref().unwrap_or_default();
    if environments.is_empty() || environments.iter().any(|env| env == "local") {
        "local"
    } else {
        &environments[0]
    }
}

fn ask_selection(prompt: &dyn Prompter, config: Config, deps: &Deps) -> Result<SwitchRequest> {
    let client = ask_choice_or_value(
        prompt,
        "Client",
        config.clients.as_deref().unwrap_or_default(),
        None,
    )?;
    let env = ask_choice_or_value(
        prompt,
        "Environment",
        config.environments.as_deref().unwrap_or_default(),
        Some(default_environment(&config)),
    )?;
    let target_name = if config.targets.len() == 1 {
        "all".to_string()
    } else {
        let targets: Vec<_> = std::iter::once("all".to_string())
            .chain(config.targets.iter().map(|target| target.name.clone()))
            .collect();
        let index = select_or_auto(prompt, "Target", targets.clone(), Some(0))?;
        targets[index].clone()
    };
    let dry_run = select_or_auto(
        prompt,
        "Mode",
        vec!["Dry run".to_string(), "Write .env files".to_string()],
        Some(1),
    )? == 0;
    Ok(SwitchRequest {
        config,
        root: deps.root.clone(),
        cwd: deps.cwd.clone(),
        env,
        client,
        target_name,
        dry_run,
    })
}

/// TS `runGuidedSwitch(deps)` (01 §5.1–5.2). Vault scans follow `ports` thread rules.
pub(crate) fn run_guided(deps: &mut Deps) -> Result<()> {
    if !(deps.stdin_is_tty && deps.stdout_is_tty) && deps.prompter.is_none() {
        return fail(
            "plain envswitch requires an interactive terminal; use envswitch switch --vault <vault> --config-item <item> --client <client> for scripts",
        );
    }
    let prompt = deps.prompter.take();
    let result = run_with_prompt(prompt.as_deref().unwrap_or(&InquirePrompter), deps);
    deps.prompter = prompt;
    result
}

fn run_with_prompt(prompt: &dyn Prompter, deps: &mut Deps) -> Result<()> {
    write_line(deps, "envswitch guided switch")?;
    write_line(deps, format!("Project root: {}", deps.root.display()))?;
    let vault = ask_vault(prompt, deps)?;
    let item = ask_config_item(prompt, &vault.items)?;
    let config = load_config(vault.op.as_ref(), item.key(), item.display_name(), deps)?;
    let req = ask_selection(prompt, config, deps)?;
    let results = switch_environment(vault.op.as_ref(), &req, &mut |event| {
        log_progress(deps, event)
    })?;
    print_switch_results(&results, &req.root, req.dry_run, deps)
}
