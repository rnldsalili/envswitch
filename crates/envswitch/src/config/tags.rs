//! Tag templates (contract 02 §3). Owner: C1. Signatures frozen by S1.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use std::path::Path;

use crate::model::{Config, TagParams, TagValues};

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

/// TS `renderTemplate(template, ctx)` (02 §3.1): single pass over
/// `\{([A-Za-z0-9_-]+)\}`; the six known keys substitute, every other key
/// (including JS prototype names, DIVERGENCES a1) renders `""`.
pub fn render_template(template: &str, ctx: &TemplateContext<'_>) -> String {
    todo!("C1: 02 §3.1")
}

/// TS `tagValues(config, params)` (02 §3.2). `cwd` replaces
/// `process.cwd()` for the `{project}` fallback: `config.project.id`
/// (even `""`) else `jsutil::node_basename(cwd)`.
pub fn tag_values(config: &Config, params: &TagParams<'_>, cwd: &Path) -> TagValues {
    todo!("C1: 02 §3.2")
}

/// TS `candidateListTags(config, env)` (02 §3.4):
/// `compact_unique([...base, project, env])` rendered with `{env}` only.
pub fn candidate_list_tags(config: &Config, env: &str, cwd: &Path) -> Vec<String> {
    todo!("C1: 02 §3.4")
}

/// TS `compactUnique(values)` (02 §3.3): drop `None` and `""`, then dedupe
/// keeping the first occurrence, order preserved.
pub fn compact_unique<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = Option<S>>,
    S: AsRef<str>,
{
    todo!("C1: 02 §3.3")
}
