//! Tag matcher (contract 02 §3.5, §4.1–4.2). Owner: C3.
//! Everything here is `pub(crate)`.
//!
//! Building a matcher has two steps so the classification logic can be tested
//! without the config module:
//!
//! 1. [`render_matcher_tags`] renders every tag the matcher needs through
//!    `config::tag_values` / `config::compact_unique`, exactly like TS
//!    `buildTagMatcher` (`switcher.ts:144-177`).
//! 2. [`TagMatcher::from_tags`] turns the rendered strings into the matcher,
//!    compiling the three scope patterns (`switcher.ts:282-286`).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::{Regex, RegexBuilder};

use crate::config::{compact_unique, tag_values};
use crate::model::{Config, ItemSummary, LayerName, TagParams};

/// Rendered into a scope template, then replaced with [`SCOPE_WILDCARD`] in
/// the scope regex (02 §3.5 erratum).
pub(crate) const SCOPE_SENTINEL: &str = "__ENVSWITCH_SCOPE__";

/// JS `.+` without flags: one or more UTF-16 units that are not one of the
/// four JS line terminators. Rust `.` only excludes `\n`, so the class is
/// spelled out (02 §3.5 erratum, plan review 04 finding 8). It runs on
/// [`code_unit_string`]s, so each matched char is exactly one UTF-16 unit.
const SCOPE_WILDCARD: &str = r"[^\r\n\x{2028}\x{2029}]+";

/// Surrogate unit `u` (0xD800..=0xDFFF) is represented by the scalar
/// `SURROGATE_BASE + (u - 0xD800)`, i.e. U+10D800..=U+10DFFF. That range lies
/// inside Supplementary Private Use Area-B (U+100000..=U+10FFFD), so every
/// value is a valid scalar, none is a line terminator, and none can collide
/// with a BMP unit (BMP units map to themselves).
const SURROGATE_BASE: u32 = 0x10_D800;

/// The tags a [`TagMatcher`] is built from, already rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MatcherTags {
    /// `compactUnique([...base, project, env])`.
    pub(crate) common_tags: Vec<String>,
    /// Target template rendered with `{target}` = [`SCOPE_SENTINEL`].
    pub(crate) target_scope: String,
    /// Client template rendered with `{client}` = [`SCOPE_SENTINEL`].
    pub(crate) client_scope: String,
    /// Group template rendered with `{group}` = [`SCOPE_SENTINEL`].
    pub(crate) group_scope: String,
    /// `tagValues({client}).client`; `None` when absent or `""` (falsy).
    pub(crate) current_client_tag: Option<String>,
    /// Per configured target: `compactUnique([targetTag(name)])`.
    pub(crate) target_tags_by_target: Vec<(String, Vec<String>)>,
    /// Per configured target: `compactUnique(groups.map(groupTag))`.
    pub(crate) group_tags_by_target: Vec<(String, Vec<String>)>,
    /// `layerTags`: only layers whose rendered tag is non-empty.
    pub(crate) layer_tags: Vec<(LayerName, String)>,
}

/// TS `TagMatcher`, built once per switch and shared by all targets.
#[derive(Debug, Clone)]
pub(crate) struct TagMatcher {
    common_tags: Vec<String>,
    target_tag_pattern: ScopePattern,
    client_tag_pattern: ScopePattern,
    group_tag_pattern: ScopePattern,
    current_client_tag: Option<String>,
    current_target_tags_by_target: HashMap<String, HashSet<String>>,
    current_group_tags_by_target: HashMap<String, HashSet<String>>,
    layer_tags: HashMap<LayerName, String>,
}

impl TagMatcher {
    /// Builds the matcher from rendered tags (the second half of TS
    /// `buildTagMatcher`).
    pub(crate) fn from_tags(tags: MatcherTags) -> TagMatcher {
        TagMatcher {
            common_tags: tags.common_tags,
            target_tag_pattern: scope_tag_pattern(&tags.target_scope),
            client_tag_pattern: scope_tag_pattern(&tags.client_scope),
            group_tag_pattern: scope_tag_pattern(&tags.group_scope),
            current_client_tag: tags.current_client_tag.filter(|tag| !tag.is_empty()),
            current_target_tags_by_target: into_tag_sets(tags.target_tags_by_target),
            current_group_tags_by_target: into_tag_sets(tags.group_tags_by_target),
            layer_tags: tags
                .layer_tags
                .into_iter()
                .filter(|(_, tag)| !tag.is_empty())
                .collect(),
        }
    }

    /// `hasLayerTag(matcher, tags, layer)`.
    fn has_layer_tag(&self, tags: &HashSet<&str>, layer: LayerName) -> bool {
        self.layer_tags
            .get(&layer)
            .is_some_and(|tag| tags.contains(tag.as_str()))
    }
}

/// `new Map(entries)` where each value is a `Set`: a later duplicate target
/// name replaces an earlier one (cannot happen after config validation).
fn into_tag_sets(entries: Vec<(String, Vec<String>)>) -> HashMap<String, HashSet<String>> {
    entries
        .into_iter()
        .map(|(target, tags)| (target, tags.into_iter().collect()))
        .collect()
}

/// TS `buildTagMatcher(config, env, client)`. `cwd` stands in for
/// `process.cwd()` in the `{project}` fallback.
pub(crate) fn build_tag_matcher(
    config: &Config,
    env: &str,
    client: &str,
    cwd: &Path,
) -> TagMatcher {
    TagMatcher::from_tags(render_matcher_tags(config, env, client, cwd))
}

/// Renders every tag `buildTagMatcher` uses (`switcher.ts:144-177`).
pub(crate) fn render_matcher_tags(
    config: &Config,
    env: &str,
    client: &str,
    cwd: &Path,
) -> MatcherTags {
    let common = tag_values(
        config,
        &TagParams {
            env: Some(env),
            ..TagParams::default()
        },
        cwd,
    );

    let mut layer_tags = Vec::new();
    for layer in LayerName::ALL {
        let rendered = tag_values(
            config,
            &TagParams {
                layer: Some(layer),
                ..TagParams::default()
            },
            cwd,
        )
        .layer;
        if let Some(tag) = rendered.filter(|tag| !tag.is_empty()) {
            layer_tags.push((layer, tag));
        }
    }

    let mut target_tags_by_target = Vec::new();
    let mut group_tags_by_target = Vec::new();
    for target in &config.targets {
        let target_tag = tag_values(
            config,
            &TagParams {
                target: Some(target.name.as_str()),
                ..TagParams::default()
            },
            cwd,
        )
        .target;
        target_tags_by_target.push((target.name.clone(), compact_unique([target_tag])));

        let group_tags = target.groups.iter().flatten().map(|group| {
            tag_values(
                config,
                &TagParams {
                    group: Some(group.as_str()),
                    ..TagParams::default()
                },
                cwd,
            )
            .group
        });
        group_tags_by_target.push((target.name.clone(), compact_unique(group_tags)));
    }

    let mut common_values: Vec<Option<String>> = common.base.into_iter().map(Some).collect();
    common_values.push(common.project);
    common_values.push(common.env);

    let current_client_tag = tag_values(
        config,
        &TagParams {
            client: Some(client),
            ..TagParams::default()
        },
        cwd,
    )
    .client;

    MatcherTags {
        common_tags: compact_unique(common_values),
        target_scope: render_scope(config, Scope::Target, cwd),
        client_scope: render_scope(config, Scope::Client, cwd),
        group_scope: render_scope(config, Scope::Group, cwd),
        current_client_tag,
        target_tags_by_target,
        group_tags_by_target,
        layer_tags,
    }
}

#[derive(Debug, Clone, Copy)]
enum Scope {
    Client,
    Target,
    Group,
}

/// `tagValues(config, { [scope]: SCOPE_SENTINEL })[scope] ?? ""`.
fn render_scope(config: &Config, scope: Scope, cwd: &Path) -> String {
    let mut params = TagParams::default();
    match scope {
        Scope::Client => params.client = Some(SCOPE_SENTINEL),
        Scope::Target => params.target = Some(SCOPE_SENTINEL),
        Scope::Group => params.group = Some(SCOPE_SENTINEL),
    }
    let values = tag_values(config, &params, cwd);
    let rendered = match scope {
        Scope::Client => values.client,
        Scope::Target => values.target,
        Scope::Group => values.group,
    };
    rendered.unwrap_or_default()
}

/// A compiled scope pattern that matches like a JS `RegExp` without the `u`
/// flag: on UTF-16 code units, so a wildcard can take half of a surrogate
/// pair (`scope/{client}{client}` matches `scope/😀` in TS).
#[derive(Debug, Clone)]
pub(crate) struct ScopePattern(Regex);

impl ScopePattern {
    /// `pattern.test(tag)`.
    pub(crate) fn is_match(&self, tag: &str) -> bool {
        self.0.is_match(&code_unit_string(tag))
    }
}

/// Re-encodes `s` so that each UTF-16 code unit is one `char`: BMP units map
/// to themselves, surrogate units to U+10D800..=U+10DFFF
/// ([`SURROGATE_BASE`]). The mapping is injective, so literal matching is
/// unchanged, while `[^…]` classes now consume one unit at a time.
fn code_unit_string(s: &str) -> Cow<'_, str> {
    if s.chars().all(|c| u32::from(c) <= 0xFFFF) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.encode_utf16()
            .map(|unit| {
                let unit = u32::from(unit);
                let scalar = if (0xD800..=0xDFFF).contains(&unit) {
                    SURROGATE_BASE + (unit - 0xD800)
                } else {
                    unit
                };
                char::from_u32(scalar)
                    .expect("non-surrogate BMP units and U+10D800..=U+10DFFF are scalars")
            })
            .collect(),
    )
}

/// TS `scopeTagPattern` after rendering: escape every regex metacharacter,
/// replace every sentinel with [`SCOPE_WILDCARD`], and anchor with `^…$`.
///
/// Splitting the rendered template on the sentinel and escaping each literal
/// part is the same as TS's escape-then-`replaceAll`: the sentinel is pure
/// `[A-Z_]`, which escaping never touches. Literal parts are converted with
/// [`code_unit_string`] like the tags they are matched against. The
/// compiled-size limit is lifted because JS has none: the pattern is a plain
/// literal with wildcard gaps, so syntax errors are impossible and only an
/// absurdly large template could fail.
pub(crate) fn scope_tag_pattern(rendered: &str) -> ScopePattern {
    let parts: Vec<String> = rendered
        .split(SCOPE_SENTINEL)
        .map(|part| regex::escape(&code_unit_string(part)))
        .collect();
    let pattern = format!("^{}$", parts.join(SCOPE_WILDCARD));
    let regex = RegexBuilder::new(&pattern)
        .size_limit(usize::MAX)
        .build()
        .expect("an escaped literal scope pattern always compiles");
    ScopePattern(regex)
}

/// TS `classifyItemWithMatcher(item, target, matcher)` (02 §4.2). Branches
/// are tested in order; the first match wins.
pub(crate) fn classify_with_matcher(
    item: &ItemSummary,
    target: &str,
    matcher: &TagMatcher,
) -> Option<LayerName> {
    let tags: HashSet<&str> = item.tags.iter().map(String::as_str).collect();
    if matcher
        .common_tags
        .iter()
        .any(|required| !tags.contains(required.as_str()))
    {
        return None;
    }

    let has_any_target = has_matching_tag(&tags, &matcher.target_tag_pattern);
    let has_any_client = has_matching_tag(&tags, &matcher.client_tag_pattern);
    let has_any_group = has_matching_tag(&tags, &matcher.group_tag_pattern);
    let has_current_target = has_any(&tags, matcher.current_target_tags_by_target.get(target));
    let has_current_group = has_any(&tags, matcher.current_group_tags_by_target.get(target));
    let has_current_client = matcher
        .current_client_tag
        .as_deref()
        .is_some_and(|tag| tags.contains(tag));

    if matcher.has_layer_tag(&tags, LayerName::Shared)
        && !has_any_target
        && !has_any_client
        && !has_any_group
    {
        return Some(LayerName::Shared);
    }
    if matcher.has_layer_tag(&tags, LayerName::Group)
        && has_current_group
        && !has_any_target
        && !has_any_client
    {
        return Some(LayerName::Group);
    }
    if matcher.has_layer_tag(&tags, LayerName::Target)
        && has_current_target
        && !has_any_client
        && !has_any_group
    {
        return Some(LayerName::Target);
    }
    if matcher.has_layer_tag(&tags, LayerName::Client) && has_current_client && !has_any_target {
        return Some(LayerName::Client);
    }
    if matcher.has_layer_tag(&tags, LayerName::ClientTarget)
        && has_current_client
        && has_current_group
        && !has_any_target
    {
        return Some(LayerName::ClientTarget);
    }
    None
}

/// `hasAny(tags, candidates ?? new Set())`.
fn has_any(tags: &HashSet<&str>, candidates: Option<&HashSet<String>>) -> bool {
    candidates.is_some_and(|candidates| {
        candidates
            .iter()
            .any(|candidate| tags.contains(candidate.as_str()))
    })
}

/// `hasMatchingTag(tags, pattern)`.
fn has_matching_tag(tags: &HashSet<&str>, pattern: &ScopePattern) -> bool {
    tags.iter().any(|tag| pattern.is_match(tag))
}
