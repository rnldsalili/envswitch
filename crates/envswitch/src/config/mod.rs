//! Config parsing and selection (contract 02 §1, §2, §3). Owner: C1.
//! Signatures and re-exports frozen by S1.

pub mod markers;
pub mod paths;
pub mod tags;

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value};

use crate::error::{Result, fail};
use crate::jsutil::{is_identifier, js_trim, parse_json};
use crate::model::{Config, ProjectConfig, ResolvedTarget, RootMarker, TagTemplates, TargetConfig};

pub use markers::validate_project_root;
pub use paths::resolve_output_path;
pub use tags::{TemplateContext, candidate_list_tags, compact_unique, render_template, tag_values};

/// TS `parseConfig(raw)` (02 §1.1–1.3). Manual `serde_json::Value` walk in
/// the exact TS check order; first failure wins; verbatim messages as
/// `Error::User`. JSON `null` is never "absent".
pub fn parse_config(raw: &str) -> Result<Config> {
    let parsed = parse_json(raw, "config field")?;
    let Value::Object(parsed) = parsed else {
        return fail("config JSON must be an object");
    };

    let targets = match parsed.get("targets") {
        Some(Value::Array(targets)) if !targets.is_empty() => targets,
        _ => return fail("config.targets must be a non-empty array"),
    };
    let targets = targets
        .iter()
        .enumerate()
        .map(|(index, target)| parse_target(target, index))
        .collect::<Result<Vec<_>>>()?;
    let project = parse_project(parsed.get("project"))?;
    let clients = parse_string_array(parsed.get("clients"), "config.clients", false)?;
    let environments =
        parse_string_array(parsed.get("environments"), "config.environments", false)?;
    let tags = parse_tags(parsed.get("tags"))?;

    let mut names = HashSet::new();
    for target in &targets {
        if !names.insert(target.name.as_str()) {
            return fail(format!(
                "config.targets contains duplicate target {}",
                target.name
            ));
        }
    }

    Ok(Config {
        project,
        clients,
        environments,
        targets,
        tags,
    })
}

/// TS `validateSelection(config, env, client)` (02 §1.5): identifier checks
/// (env, then client), then membership when the list is present and
/// non-empty.
pub fn validate_selection(config: &Config, env: &str, client: &str) -> Result<()> {
    validate_identifier(env, "environment")?;
    validate_identifier(client, "client")?;

    if let Some(environments) = config.environments.as_ref().filter(|list| !list.is_empty())
        && !environments.iter().any(|candidate| candidate == env)
    {
        return fail(format!(
            "environment {env} is not listed in config.environments"
        ));
    }
    if let Some(clients) = config.clients.as_ref().filter(|list| !list.is_empty())
        && !clients.iter().any(|candidate| candidate == client)
    {
        return fail(format!("client {client} is not listed in config.clients"));
    }
    Ok(())
}

/// TS `selectTargets(config, targetName)` (02 §1.5): `"all"` returns every
/// target in config order without an identifier check; otherwise identifier
/// check, then the first exact match or `target <name> is not defined in
/// config.targets`.
pub fn select_targets(config: &Config, target_name: &str) -> Result<Vec<TargetConfig>> {
    if target_name == "all" {
        return Ok(config.targets.clone());
    }
    validate_identifier(target_name, "target")?;
    match config
        .targets
        .iter()
        .find(|candidate| candidate.name == target_name)
    {
        Some(target) => Ok(vec![target.clone()]),
        None => fail(format!(
            "target {target_name} is not defined in config.targets"
        )),
    }
}

/// TS `resolveTargets(root, targets)`: [`resolve_output_path`] for each
/// target in order; first failure wins.
pub fn resolve_targets(root: &Path, targets: &[TargetConfig]) -> Result<Vec<ResolvedTarget>> {
    targets
        .iter()
        .map(|target| {
            Ok(ResolvedTarget {
                target: target.clone(),
                output_path: resolve_output_path(root, &target.output)?,
            })
        })
        .collect()
}

/// TS `validateIdentifier(value, name)`.
fn validate_identifier(value: &str, name: &str) -> Result<()> {
    if is_identifier(value) {
        Ok(())
    } else {
        fail(format!("{name} contains unsupported characters"))
    }
}

/// `typeof value === "string" && value.trim() !== ""`.
fn non_empty_string(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::String(text)) if !js_trim(text).is_empty() => Some(text),
        _ => None,
    }
}

/// TS `parseTarget(value, index)`.
fn parse_target(value: &Value, index: usize) -> Result<TargetConfig> {
    let Value::Object(value) = value else {
        return fail(format!("config.targets[{index}] must be an object"));
    };
    let Some(name) = non_empty_string(value.get("name")) else {
        return fail(format!(
            "config.targets[{index}].name must be a non-empty string"
        ));
    };
    let Some(output) = non_empty_string(value.get("output")) else {
        return fail(format!(
            "config.targets[{index}].output must be a non-empty string"
        ));
    };

    validate_identifier(name, &format!("target {name}"))?;

    let groups = parse_string_array(
        value.get("groups"),
        &format!("config.targets[{index}].groups"),
        false,
    )?;
    if let Some(groups) = &groups {
        for group in groups {
            validate_identifier(group, &format!("target {name} group {group}"))?;
        }
    }

    Ok(TargetConfig {
        name: name.to_string(),
        output: output.to_string(),
        groups,
    })
}

/// TS `parseProject(value)`.
fn parse_project(value: Option<&Value>) -> Result<Option<ProjectConfig>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Object(value) = value else {
        return fail("config.project must be an object");
    };

    let id = match value.get("id") {
        None => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => return fail("config.project.id must be a string"),
    };
    // `if (rawId)`: an empty id is stored but never validated.
    if let Some(id) = id.as_deref().filter(|id| !id.is_empty()) {
        validate_identifier(id, "project id")?;
    }

    let root_markers = match value.get("rootMarkers") {
        None => None,
        Some(Value::Array(markers)) => Some(
            markers
                .iter()
                .enumerate()
                .map(|(index, marker)| parse_root_marker(marker, index))
                .collect::<Result<Vec<_>>>()?,
        ),
        Some(_) => return fail("config.project.rootMarkers must be an array"),
    };

    Ok(Some(ProjectConfig { id, root_markers }))
}

/// TS `parseRootMarker(value, index)`.
fn parse_root_marker(value: &Value, index: usize) -> Result<RootMarker> {
    let Value::Object(value) = value else {
        return fail(format!(
            "config.project.rootMarkers[{index}] must be an object"
        ));
    };
    let path = optional_non_empty_string(value, "path", || {
        format!("config.project.rootMarkers[{index}].path must be a non-empty string")
    })?;
    let package_json_name = optional_non_empty_string(value, "packageJsonName", || {
        format!("config.project.rootMarkers[{index}].packageJsonName must be a non-empty string")
    })?;
    // Both values are non-blank when present, so this fires only when both
    // keys are absent.
    if path.is_none() && package_json_name.is_none() {
        return fail(format!(
            "config.project.rootMarkers[{index}] must define path or packageJsonName"
        ));
    }
    Ok(RootMarker {
        path,
        package_json_name,
    })
}

/// `if (value[key] !== undefined) { if (typeof ... !== "string" ||
/// ....trim() === "") fail(message) }`.
fn optional_non_empty_string(
    object: &Map<String, Value>,
    key: &str,
    message: impl FnOnce() -> String,
) -> Result<Option<String>> {
    match object.get(key) {
        None => Ok(None),
        present => match non_empty_string(present) {
            Some(text) => Ok(Some(text.to_string())),
            None => fail(message()),
        },
    }
}

/// TS `parseTags(value)`.
fn parse_tags(value: Option<&Value>) -> Result<Option<TagTemplates>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Value::Object(value) = value else {
        return fail("config.tags must be an object");
    };

    let mut tags = TagTemplates {
        // Only parsed when present, so the `is required` branch is dead.
        base: match value.get("base") {
            None => None,
            base => parse_string_array(base, "config.tags.base", true)?,
        },
        ..TagTemplates::default()
    };
    let slots: [(&str, &mut Option<String>); 6] = [
        ("project", &mut tags.project),
        ("env", &mut tags.env),
        ("client", &mut tags.client),
        ("target", &mut tags.target),
        ("group", &mut tags.group),
        ("layer", &mut tags.layer),
    ];
    for (key, slot) in slots {
        *slot = optional_non_empty_string(value, key, || {
            format!("config.tags.{key} must be a non-empty string")
        })?;
    }
    Ok(Some(tags))
}

/// TS `parseStringArray(value, name, required)`.
fn parse_string_array(
    value: Option<&Value>,
    name: &str,
    required: bool,
) -> Result<Option<Vec<String>>> {
    let Some(value) = value else {
        if required {
            return fail(format!("{name} is required"));
        }
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail(format!("{name} must be an array"));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match non_empty_string(Some(item)) {
            Some(text) => Ok(text.to_string()),
            None => fail(format!("{name}[{index}] must be a non-empty string")),
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}
