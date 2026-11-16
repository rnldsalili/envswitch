//! Tag templates (contract 02 §3). Owner: C1. Signatures frozen by S1.

use std::collections::HashSet;
use std::path::Path;

use crate::jsutil::node_basename;
use crate::model::{Config, TagParams, TagTemplates, TagValues};

/// Default `tags.project` template.
pub const DEFAULT_PROJECT_TEMPLATE: &str = "project:{project}";
/// Default `tags.env` template.
pub const DEFAULT_ENV_TEMPLATE: &str = "env:{env}";
/// Default `tags.client` template.
pub const DEFAULT_CLIENT_TEMPLATE: &str = "client:{client}";
/// Default `tags.target` template.
pub const DEFAULT_TARGET_TEMPLATE: &str = "target:{target}";
/// Default `tags.group` template.
pub const DEFAULT_GROUP_TEMPLATE: &str = "group:{group}";
/// Default `tags.layer` template.
pub const DEFAULT_LAYER_TEMPLATE: &str = "layer:{layer}";
// Default `tags.base` is `[]`.

/// Placeholder values for [`render_template`]. Absent params are `""`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TemplateContext<'a> {
    pub project: &'a str,
    pub env: &'a str,
    pub client: &'a str,
    pub target: &'a str,
    pub group: &'a str,
    pub layer: &'a str,
}

impl TemplateContext<'_> {
    /// `context[key] ?? ""` for an own key; JS prototype names such as
    /// `constructor` also render `""` (DIVERGENCES a1).
    fn lookup(&self, key: &str) -> &str {
        match key {
            "project" => self.project,
            "env" => self.env,
            "client" => self.client,
            "target" => self.target,
            "group" => self.group,
            "layer" => self.layer,
            _ => "",
        }
    }
}

/// `[A-Za-z0-9_-]`, the placeholder name class.
fn is_placeholder_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// TS `renderTemplate(template, ctx)` (02 §3.1): single pass over
/// `\{([A-Za-z0-9_-]+)\}`; the six known keys substitute, every other key
/// (including JS prototype names, DIVERGENCES a1) renders `""`.
pub fn render_template(template: &str, ctx: &TemplateContext<'_>) -> String {
    let bytes = template.as_bytes();
    let mut rendered = String::with_capacity(template.len());
    // Start of the not-yet-copied literal text.
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        let name_start = index + 1;
        let mut name_end = name_start;
        while name_end < bytes.len() && is_placeholder_byte(bytes[name_end]) {
            name_end += 1;
        }
        if name_end > name_start && bytes.get(name_end) == Some(&b'}') {
            rendered.push_str(&template[copied..index]);
            rendered.push_str(ctx.lookup(&template[name_start..name_end]));
            index = name_end + 1;
            copied = index;
        } else {
            // No match at this `{`; the regex retries from the next char.
            index += 1;
        }
    }
    rendered.push_str(&template[copied..]);
    rendered
}

/// TS `tagValues(config, params)` (02 §3.2). `cwd` replaces
/// `process.cwd()` for the `{project}` fallback: `config.project.id`
/// (even `""`) else `jsutil::node_basename(cwd)`.
pub fn tag_values(config: &Config, params: &TagParams<'_>, cwd: &Path) -> TagValues {
    // `{ ...DEFAULT_TAGS, ...(config.tags ?? {}) }`
    let no_tags = TagTemplates::default();
    let tags = config.tags.as_ref().unwrap_or(&no_tags);
    let project_id = config
        .project
        .as_ref()
        .and_then(|project| project.id.as_deref());
    let fallback_project;
    let project = match project_id {
        Some(id) => id,
        None => {
            fallback_project = node_basename(cwd);
            fallback_project.as_str()
        }
    };
    let layer = params.layer.map(|layer| layer.as_str());
    let context = TemplateContext {
        project,
        env: params.env.unwrap_or(""),
        client: params.client.unwrap_or(""),
        target: params.target.unwrap_or(""),
        group: params.group.unwrap_or(""),
        layer: layer.unwrap_or(""),
    };
    // `params.x ? renderTemplate(templates.x, context) : undefined`
    let render_if =
        |value: Option<&str>, custom: &Option<String>, default: &str| -> Option<String> {
            let template = custom.as_deref().unwrap_or(default);
            value
                .filter(|value| !value.is_empty())
                .map(|_| render_template(template, &context))
        };

    let base = tags
        .base
        .iter()
        .flatten()
        .map(|template| render_template(template, &context))
        .filter(|tag| !tag.is_empty())
        .collect();

    TagValues {
        base,
        project: render_if(project_id, &tags.project, DEFAULT_PROJECT_TEMPLATE),
        env: render_if(params.env, &tags.env, DEFAULT_ENV_TEMPLATE),
        client: render_if(params.client, &tags.client, DEFAULT_CLIENT_TEMPLATE),
        target: render_if(params.target, &tags.target, DEFAULT_TARGET_TEMPLATE),
        group: render_if(params.group, &tags.group, DEFAULT_GROUP_TEMPLATE),
        layer: render_if(layer, &tags.layer, DEFAULT_LAYER_TEMPLATE),
    }
}

/// TS `candidateListTags(config, env)` (02 §3.4):
/// `compact_unique([...base, project, env])` rendered with `{env}` only.
pub fn candidate_list_tags(config: &Config, env: &str, cwd: &Path) -> Vec<String> {
    let params = TagParams {
        env: Some(env),
        ..TagParams::default()
    };
    let tags = tag_values(config, &params, cwd);
    compact_unique(
        tags.base
            .into_iter()
            .map(Some)
            .chain([tags.project, tags.env]),
    )
}

/// TS `compactUnique(values)` (02 §3.3): drop `None` and `""`, then dedupe
/// keeping the first occurrence, order preserved.
pub fn compact_unique<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = Option<S>>,
    S: AsRef<str>,
{
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for value in values.into_iter().flatten() {
        let value = value.as_ref();
        if !value.is_empty() && seen.insert(value.to_string()) {
            unique.push(value.to_string());
        }
    }
    unique
}
