//! C3 unit tests that run without the config (C1) and op/dotenv (C2) bodies.
//!
//! Matchers are built from hand-rendered tags ([`MatcherTags`]) equal to what
//! `render_matcher_tags` produces for the TS test config with the default
//! templates, so classification, merge, and collation are exercised directly.
//! The same behaviors are covered end to end through the public API in
//! `tests/switcher_*.rs` (merge gate).
//!
//! Expected values come from the TS oracle (`src/switcher.ts` at `53de029`,
//! Bun 1.4.0, `LANG=en_US.UTF-8`), printed by `tests/switcher_oracle/scenarios.ts` and
//! `tests/switcher_oracle/collation_golden.ts`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use super::collation_golden::{FULL_ENV, GROUPS, PAIRS, TITLES};
use super::matcher::{
    MatcherTags, SCOPE_SENTINEL, TagMatcher, classify_with_matcher, scope_tag_pattern,
};
use super::merge::{READ_CONCURRENCY, compare_titles, merge_layers, merge_target, read_in_order};
use crate::error::{Error, Result};
use crate::model::{
    EnvLayer, EnvMap, ItemSummary, LayerName, ProgressEvent, ResolvedTarget, TargetConfig,
};
use crate::ports::OnePassword;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn item(id: &str, title: &str, tags: &[&str]) -> ItemSummary {
    ItemSummary {
        id: id.to_string(),
        title: title.to_string(),
        tags: strings(tags),
    }
}

fn env(pairs: &[(&str, &str)]) -> EnvMap {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

const COMMON: [&str; 3] = ["envswitch", "project:demo", "env:local"];

fn tagged(id: &str, extra: &[&str]) -> ItemSummary {
    let mut tags = COMMON.to_vec();
    tags.extend_from_slice(extra);
    item(id, id, &tags)
}

fn default_layer_tags() -> Vec<(LayerName, String)> {
    LayerName::ALL
        .iter()
        .map(|layer| (*layer, format!("layer:{layer}")))
        .collect()
}

/// `render_matcher_tags` output for the TS test config (`switcher.test.ts`):
/// project `demo`, base `["envswitch"]`, targets `api` (groups `apis`) and
/// `web` (groups `apps`), default templates, env `local`.
fn ts_config_tags(client: &str) -> MatcherTags {
    MatcherTags {
        common_tags: strings(&COMMON),
        target_scope: format!("target:{SCOPE_SENTINEL}"),
        client_scope: format!("client:{SCOPE_SENTINEL}"),
        group_scope: format!("group:{SCOPE_SENTINEL}"),
        current_client_tag: Some(format!("client:{client}")),
        target_tags_by_target: vec![
            ("api".to_string(), strings(&["target:api"])),
            ("web".to_string(), strings(&["target:web"])),
        ],
        group_tags_by_target: vec![
            ("api".to_string(), strings(&["group:apis"])),
            ("web".to_string(), strings(&["group:apps"])),
        ],
        layer_tags: default_layer_tags(),
    }
}

fn ts_matcher() -> TagMatcher {
    TagMatcher::from_tags(ts_config_tags("alpha"))
}

/// The 16 items of `switcher.test.ts`, in its order.
fn ts_items() -> Vec<ItemSummary> {
    vec![
        tagged("shared", &["layer:shared"]),
        tagged("api-group", &["layer:group", "group:apis"]),
        tagged("app-group", &["layer:group", "group:apps"]),
        tagged("shared-with-group", &["layer:shared", "group:apis"]),
        tagged("api-target", &["layer:target", "target:api"]),
        tagged("web-target", &["layer:target", "target:web"]),
        tagged(
            "target-with-group",
            &["layer:target", "target:api", "group:apis"],
        ),
        tagged(
            "target-with-client",
            &["layer:target", "target:api", "client:alpha"],
        ),
        tagged("alpha", &["layer:client", "client:alpha"]),
        tagged(
            "alpha-web",
            &["layer:client-target", "client:alpha", "group:apps"],
        ),
        tagged(
            "alpha-api",
            &["layer:client-target", "client:alpha", "group:apis"],
        ),
        tagged(
            "beta-web",
            &["layer:client-target", "client:beta", "group:apps"],
        ),
        tagged(
            "alpha-target-web",
            &["layer:client-target", "client:alpha", "target:web"],
        ),
        tagged("shared-unknown-client", &["layer:shared", "client:gamma"]),
        tagged("shared-unknown-target", &["layer:shared", "target:worker"]),
        tagged("shared-unknown-group", &["layer:shared", "group:workers"]),
    ]
}

/// The `values` map of `switcher.test.ts`.
fn ts_values() -> HashMap<String, EnvMap> {
    let mut values = HashMap::new();
    let mut put = |id: &str, pairs: &[(&str, &str)]| {
        values.insert(id.to_string(), env(pairs));
    };
    put("shared", &[("SHARED", "1"), ("COMMON", "shared")]);
    put("api-group", &[("API_GROUP", "1"), ("COMMON", "api-group")]);
    put("app-group", &[("APP_GROUP", "1"), ("COMMON", "app-group")]);
    put("shared-with-group", &[("SHOULD_NOT_APPLY", "1")]);
    put("api-target", &[("API_ONLY", "1"), ("COMMON", "api")]);
    put("web-target", &[("WEB_ONLY", "1"), ("COMMON", "web")]);
    put("target-with-group", &[("SHOULD_NOT_APPLY", "1")]);
    put("target-with-client", &[("SHOULD_NOT_APPLY", "1")]);
    put("alpha", &[("CLIENT_ONLY", "1"), ("COMMON", "alpha")]);
    put(
        "alpha-web",
        &[("WEB_CLIENT_ONLY", "1"), ("COMMON", "alpha-web")],
    );
    put(
        "alpha-api",
        &[("API_CLIENT_ONLY", "1"), ("COMMON", "alpha-api")],
    );
    for id in [
        "beta-web",
        "alpha-target-web",
        "shared-unknown-client",
        "shared-unknown-target",
        "shared-unknown-group",
    ] {
        put(id, &[("SHOULD_NOT_APPLY", "1")]);
    }
    values
}

fn resolved(name: &str) -> ResolvedTarget {
    ResolvedTarget {
        target: TargetConfig {
            name: name.to_string(),
            output: format!("apps/{name}/.env"),
            groups: None,
        },
        output_path: PathBuf::from(format!("/project/apps/{name}/.env")),
    }
}

/// Minimal [`OnePassword`] for unit tests: scripted values by `key()`,
/// optional failures, and a read log.
struct UnitOp {
    values: HashMap<String, EnvMap>,
    failing: Vec<String>,
    reads: Mutex<Vec<String>>,
}

impl UnitOp {
    fn new(values: HashMap<String, EnvMap>) -> Self {
        UnitOp {
            values,
            failing: Vec::new(),
            reads: Mutex::new(Vec::new()),
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

impl OnePassword for UnitOp {
    fn assert_authenticated(&self) -> Result<()> {
        unreachable!("merge never authenticates")
    }

    fn list_items_by_tags(&self, _tags: &[String]) -> Result<Vec<ItemSummary>> {
        unreachable!("merge never lists")
    }

    fn read_config_item(&self, _item: &str) -> Result<String> {
        unreachable!("merge never reads config items")
    }

    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap> {
        self.reads.lock().unwrap().push(item.key().to_string());
        if self.failing.iter().any(|key| key == item.key()) {
            return Err(Error::internal("op read failed"));
        }
        Ok(self.values.get(item.key()).cloned().unwrap_or_default())
    }
}

/// Runs `merge_target` and returns the result plus `"<status>:<message>"`
/// event lines.
fn merge(
    op: &UnitOp,
    candidates: &[ItemSummary],
    target: &str,
    matcher: &TagMatcher,
    cache: &mut HashMap<String, EnvMap>,
) -> (Result<crate::model::MergeResult>, Vec<String>) {
    let mut events = Vec::new();
    let result = merge_target(
        op,
        candidates,
        &resolved(target),
        matcher,
        cache,
        &mut |event: ProgressEvent| events.push(format!("{}:{}", event.status, event.message)),
    );
    (result, events)
}

fn layer_ids(layers: &[EnvLayer]) -> Vec<String> {
    layers.iter().map(|layer| layer.item.id.clone()).collect()
}

/// Same-rank `layer:shared` layers with `{COMMON: id}` values.
fn shared_layers(items: &[(&str, &str)]) -> Vec<EnvLayer> {
    items
        .iter()
        .map(|(id, title)| EnvLayer {
            name: LayerName::Shared,
            rank: LayerName::Shared.rank(),
            item: item(id, title, &[]),
            values: env(&[("COMMON", id)]),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Classification (02 §4.2; TS rows 10-11; 02 §3.5 erratum)
// ---------------------------------------------------------------------------

#[test]
fn classifies_only_items_that_belong_to_the_selected_target() {
    // switcher.test.ts "classifies only items that belong to the selected
    // target": the same 18 assertions, item index and target as in TS.
    let items = ts_items();
    let matcher = ts_matcher();
    let cases: [(usize, &str, Option<LayerName>); 18] = [
        (0, "api", Some(LayerName::Shared)),
        (1, "api", Some(LayerName::Group)),
        (2, "api", None),
        (2, "web", Some(LayerName::Group)),
        (3, "api", None),
        (4, "api", Some(LayerName::Target)),
        (5, "api", None),
        (6, "api", None),
        (7, "api", None),
        (8, "api", Some(LayerName::Client)),
        (9, "api", None),
        (9, "web", Some(LayerName::ClientTarget)),
        (10, "api", Some(LayerName::ClientTarget)),
        (11, "web", None),
        (12, "web", None),
        (13, "api", None),
        (14, "api", None),
        (15, "api", None),
    ];
    for (index, target, expected) in cases {
        assert_eq!(
            classify_with_matcher(&items[index], target, &matcher),
            expected,
            "item {} for target {target}",
            items[index].id
        );
    }
}

#[test]
fn classification_branch_edges() {
    let matcher = ts_matcher();
    let classify =
        |extra: &[&str], target: &str| classify_with_matcher(&tagged("x", extra), target, &matcher);
    // Missing a common tag.
    assert_eq!(
        classify_with_matcher(
            &item("x", "x", &["envswitch", "env:local", "layer:shared"]),
            "api",
            &matcher
        ),
        None
    );
    // No layer tag, unknown layer tag.
    assert_eq!(classify(&[], "api"), None);
    assert_eq!(classify(&["layer:foo"], "api"), None);
    // Extra non-scope tags are fine.
    assert_eq!(
        classify(&["layer:shared", "misc"], "api"),
        Some(LayerName::Shared)
    );
    // First passing branch wins.
    assert_eq!(
        classify(&["layer:shared", "layer:client", "client:alpha"], "api"),
        Some(LayerName::Client)
    );
    // A group item with a foreign group tag still applies.
    assert_eq!(
        classify(&["layer:group", "group:apis", "group:workers"], "api"),
        Some(LayerName::Group)
    );
    // A target item with several target tags applies to each.
    for target in ["api", "web"] {
        assert_eq!(
            classify(&["layer:target", "target:api", "target:web"], target),
            Some(LayerName::Target)
        );
    }
    // A client item with group tags applies to every target.
    for target in ["api", "web"] {
        assert_eq!(
            classify(&["layer:client", "client:alpha", "group:apis"], target),
            Some(LayerName::Client)
        );
    }
    // A client item for another client does not apply.
    assert_eq!(classify(&["layer:client", "client:beta"], "api"), None);
    // An unconfigured target has no current target or group tags.
    assert_eq!(classify(&["layer:target", "target:worker"], "worker"), None);
    assert_eq!(
        classify(&["layer:shared"], "worker"),
        Some(LayerName::Shared)
    );
}

#[test]
fn absent_or_empty_current_client_never_matches() {
    let mut tags = ts_config_tags("alpha");
    tags.current_client_tag = None;
    let matcher = TagMatcher::from_tags(tags.clone());
    assert_eq!(
        classify_with_matcher(
            &tagged("x", &["layer:client", "client:alpha"]),
            "api",
            &matcher
        ),
        None
    );
    tags.current_client_tag = Some(String::new());
    let matcher = TagMatcher::from_tags(tags);
    assert_eq!(
        classify_with_matcher(&tagged("x", &["layer:client", ""]), "api", &matcher),
        None
    );
}

#[test]
fn empty_layer_tags_disable_their_layer() {
    // `tags.layer: "{env}"` renders every layer tag as "" (falsy in TS).
    let mut tags = ts_config_tags("alpha");
    tags.layer_tags = LayerName::ALL
        .iter()
        .map(|layer| (*layer, String::new()))
        .collect();
    let matcher = TagMatcher::from_tags(tags);
    for extra in [&["layer:shared"][..], &[""][..]] {
        assert_eq!(
            classify_with_matcher(&tagged("x", extra), "api", &matcher),
            None
        );
    }
}

#[test]
fn recognizes_unknown_scopes_with_custom_tag_templates() {
    // switcher.test.ts "recognizes unknown scopes with custom tag templates".
    let mut tags = ts_config_tags("alpha");
    tags.client_scope = format!("scope/client/{SCOPE_SENTINEL}");
    tags.target_scope = format!("scope/target/{SCOPE_SENTINEL}");
    tags.group_scope = format!("scope/group/{SCOPE_SENTINEL}");
    tags.current_client_tag = Some("scope/client/alpha".to_string());
    tags.target_tags_by_target = vec![("api".to_string(), strings(&["scope/target/api"]))];
    tags.group_tags_by_target = vec![("api".to_string(), strings(&["scope/group/apis"]))];
    let matcher = TagMatcher::from_tags(tags);
    for tag in [
        "scope/client/beta",
        "scope/target/worker",
        "scope/group/workers",
    ] {
        assert_eq!(
            classify_with_matcher(&tagged(tag, &["layer:shared", tag]), "api", &matcher),
            None,
            "{tag}"
        );
    }
    assert_eq!(
        classify_with_matcher(
            &tagged("x", &["layer:shared", "client:beta"]),
            "api",
            &matcher
        ),
        Some(LayerName::Shared)
    );
}

#[test]
fn scope_wildcard_excludes_exactly_the_js_line_terminators() {
    // Oracle: a `layer:shared` item whose only scope-looking tag contains CR,
    // LF, U+2028 or U+2029 after the prefix is still `shared`; NEL (U+0085)
    // and TAB are ordinary characters for JS `.`.
    let matcher = ts_matcher();
    for scope in ["client", "target", "group"] {
        for terminator in ["\r", "\n", "\u{2028}", "\u{2029}"] {
            for tag in [
                format!("{scope}:foreign{terminator}"),
                format!("{scope}:{terminator}"),
                format!("{scope}:a{terminator}b"),
                format!("{terminator}{scope}:foreign"),
            ] {
                assert_eq!(
                    classify_with_matcher(&tagged("x", &["layer:shared", &tag]), "api", &matcher),
                    Some(LayerName::Shared),
                    "{tag:?}"
                );
            }
        }
        for other in ["\u{85}", "\t"] {
            for tag in [
                format!("{scope}:foreign{other}"),
                format!("{scope}:{other}"),
                format!("{scope}:a{other}b"),
            ] {
                assert_eq!(
                    classify_with_matcher(&tagged("x", &["layer:shared", &tag]), "api", &matcher),
                    None,
                    "{tag:?}"
                );
            }
            let leading = format!("{other}{scope}:foreign");
            assert_eq!(
                classify_with_matcher(&tagged("x", &["layer:shared", &leading]), "api", &matcher),
                Some(LayerName::Shared),
                "{leading:?}"
            );
        }
        // `.+` needs at least one character.
        let bare = format!("{scope}:");
        assert_eq!(
            classify_with_matcher(&tagged("x", &["layer:shared", &bare]), "api", &matcher),
            Some(LayerName::Shared)
        );
        for tag in [format!("{scope}:foreign"), format!("{scope}:ab")] {
            assert_eq!(
                classify_with_matcher(&tagged("x", &["layer:shared", &tag]), "api", &matcher),
                None
            );
        }
    }
}

#[test]
fn scope_patterns_match_escaped_literals_around_wildcards() {
    let pattern = |template: &str| scope_tag_pattern(&template.replace("{}", SCOPE_SENTINEL));
    // Regex metacharacters are literal (oracle "metachars" cases).
    let client = pattern("c.{}+(x)");
    assert!(client.is_match("c.beta+(x)"));
    assert!(!client.is_match("cZbeta+(x)"));
    assert!(!client.is_match("c.beta+(x)x"));
    assert!(!client.is_match("c.+(x)"));
    let target = pattern("t[{}]");
    assert!(target.is_match("t[worker]]"));
    assert!(!target.is_match("t[worker]{2}"));
    assert!(!target.is_match("tworker"));
    let group = pattern("g^{}$|*?\\");
    assert!(group.is_match("g^workers$|*?\\"));
    assert!(!group.is_match("gworkers"));
    // Every sentinel becomes a wildcard.
    let multi = pattern("{}/{}");
    assert!(multi.is_match("a/b"));
    assert!(multi.is_match("a/b/c"));
    assert!(!multi.is_match("a/"));
    assert!(!multi.is_match("/b"));
    // No placeholder: a literal; bare placeholder: every non-empty tag.
    let literal = pattern("client");
    assert!(literal.is_match("client"));
    assert!(!literal.is_match("client:alpha"));
    let bare = pattern("{}");
    assert!(bare.is_match("anything at all"));
    assert!(!bare.is_match(""));
    // Empty rendered template: only the empty tag.
    let empty = scope_tag_pattern("");
    assert!(empty.is_match(""));
    assert!(!empty.is_match("x"));
    // Non-ASCII literals and wildcards.
    let unicode = pattern("клиент/{}/é");
    assert!(unicode.is_match("клиент/αβγ😀/é"));
    assert!(!unicode.is_match("клиент/αβγ/e\u{301}"));
}

#[test]
fn scope_wildcards_match_utf16_code_units_like_js() {
    // JS `RegExp` without the `u` flag: `.` takes one UTF-16 unit, so two
    // adjacent wildcards can split one emoji (U+1F600 = D83D DE00). Expected
    // values from `tests/switcher_oracle/scenarios.ts` ("utf16-*" cases).
    let pattern = |template: &str| scope_tag_pattern(&template.replace("{}", SCOPE_SENTINEL));

    let double = pattern("scope/{}{}");
    assert!(double.is_match("scope/\u{1f600}"));
    assert!(!double.is_match("scope/a"));
    assert!(double.is_match("scope/ab"));
    assert!(double.is_match("scope/\u{1f600}\u{1f600}"));
    assert!(double.is_match("scope/alphaalpha"));

    // Astral literals around a double wildcard.
    let literals = pattern("\u{1f600}/{}{}/\u{1f600}");
    assert!(literals.is_match("\u{1f600}/\u{1f600}/\u{1f600}"));
    assert!(!literals.is_match("\u{1f600}/x/\u{1f600}"));
    assert!(literals.is_match("\u{1f600}/xy/\u{1f600}"));
    assert!(!literals.is_match("\u{1f600}//\u{1f600}"));

    // A single wildcard after an astral literal.
    let single = pattern("\u{1f600}{}");
    assert!(single.is_match("\u{1f600}x"));
    assert!(!single.is_match("\u{1f600}"));
    assert!(single.is_match("\u{1f600}\u{1f680}"));

    // The private scalars that stand in for surrogate units never collide
    // with real characters: a literal U+10D83D (itself DBF5 DC3D) must not
    // match the high surrogate of U+1F600 (D83D, encoded as U+10D83D).
    let private = pattern("\u{10d83d}{}");
    assert!(!private.is_match("\u{1f600}x"));
    assert!(private.is_match("\u{10d83d}x"));
}

#[test]
fn utf16_scope_cases_classify_like_the_oracle() {
    // Oracle "utf16-double": `tags.client: "scope/{client}{client}"`.
    let mut tags = ts_config_tags("alpha");
    tags.client_scope = format!("scope/{SCOPE_SENTINEL}{SCOPE_SENTINEL}");
    tags.current_client_tag = Some("scope/alphaalpha".to_string());
    let matcher = TagMatcher::from_tags(tags);
    let classify = |extra: &[&str]| classify_with_matcher(&tagged("x", extra), "api", &matcher);
    assert_eq!(classify(&["layer:shared", "scope/\u{1f600}"]), None);
    assert_eq!(
        classify(&["layer:shared", "scope/a"]),
        Some(LayerName::Shared)
    );
    assert_eq!(
        classify(&["layer:client", "scope/alphaalpha"]),
        Some(LayerName::Client)
    );
}

#[test]
fn bare_placeholder_scope_matches_every_tag() {
    // Oracle "bare" cases: `tags.client: "{client}"`.
    let mut tags = ts_config_tags("alpha");
    tags.client_scope = SCOPE_SENTINEL.to_string();
    tags.current_client_tag = Some("alpha".to_string());
    let matcher = TagMatcher::from_tags(tags);
    let classify = |extra: &[&str]| classify_with_matcher(&tagged("x", extra), "api", &matcher);
    assert_eq!(classify(&["layer:shared"]), None);
    assert_eq!(classify(&["layer:target", "target:api"]), None);
    assert_eq!(
        classify(&["layer:client", "alpha"]),
        Some(LayerName::Client)
    );
    assert_eq!(classify(&["layer:client", "alpha", "target:api"]), None);
}

#[test]
fn huge_scope_templates_still_compile() {
    // JS has no compiled-size limit; the default `regex` limit (10 MiB)
    // would reject this template.
    let template = format!("{}{SCOPE_SENTINEL}", "x".repeat(2 * 1024 * 1024));
    let pattern = scope_tag_pattern(&template);
    assert!(pattern.is_match(&format!("{}y", "x".repeat(2 * 1024 * 1024))));
}

// ---------------------------------------------------------------------------
// Merge (02 §4.3; TS rows 12, 14, 15)
// ---------------------------------------------------------------------------

#[test]
fn dry_run_merge_for_api_matches_the_oracle() {
    // switcher.test.ts "dry run merges supported layers without writing
    // files" and "dry run logs progress" (reading events only).
    let op = UnitOp::new(ts_values());
    let mut cache = HashMap::new();
    let (result, events) = merge(&op, &ts_items(), "api", &ts_matcher(), &mut cache);
    let result = result.unwrap();
    assert_eq!(
        layer_ids(&result.layers),
        ["shared", "api-group", "api-target", "alpha", "alpha-api"]
    );
    assert_eq!(
        result
            .layers
            .iter()
            .map(|layer| (layer.name, layer.rank))
            .collect::<Vec<_>>(),
        [
            (LayerName::Shared, 1),
            (LayerName::Group, 2),
            (LayerName::Target, 3),
            (LayerName::Client, 4),
            (LayerName::ClientTarget, 5),
        ]
    );
    assert_eq!(result.values.len(), 6);
    assert_eq!(result.overwritten_keys, ["COMMON"]);
    assert_eq!(result.values["COMMON"], "alpha-api");
    assert_eq!(
        events,
        [
            "info:reading shared",
            "info:reading api-group",
            "info:reading api-target",
            "info:reading alpha",
            "info:reading alpha-api",
        ]
    );
}

#[test]
fn web_merge_matches_the_oracle_in_group_aware_layer_order() {
    // switcher.test.ts "writes merged values in group-aware layer order".
    let op = UnitOp::new(ts_values());
    let mut cache = HashMap::new();
    let (result, _) = merge(&op, &ts_items(), "web", &ts_matcher(), &mut cache);
    let result = result.unwrap();
    assert_eq!(
        layer_ids(&result.layers),
        ["shared", "app-group", "web-target", "alpha", "alpha-web"]
    );
    // JS assignment order: first insertion position is kept on overwrite.
    let merged: Vec<(&str, &str)> = result
        .values
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        merged,
        [
            ("SHARED", "1"),
            ("COMMON", "alpha-web"),
            ("APP_GROUP", "1"),
            ("WEB_ONLY", "1"),
            ("CLIENT_ONLY", "1"),
            ("WEB_CLIENT_ONLY", "1"),
        ]
    );
    assert_eq!(result.overwritten_keys, ["COMMON"]);
}

#[test]
fn caches_item_reads_across_targets() {
    // switcher.test.ts "caches 1Password item reads across targets in one
    // switch" plus the oracle's event order for `all`.
    let op = UnitOp::new(ts_values());
    let items = ts_items();
    let matcher = ts_matcher();
    let mut cache = HashMap::new();
    let (api, api_events) = merge(&op, &items, "api", &matcher, &mut cache);
    let (web, web_events) = merge(&op, &items, "web", &matcher, &mut cache);
    assert_eq!(api.unwrap().values.len(), 6);
    assert_eq!(web.unwrap().values.len(), 6);
    assert_eq!(
        api_events,
        [
            "info:reading shared",
            "info:reading api-group",
            "info:reading api-target",
            "info:reading alpha",
            "info:reading alpha-api",
        ]
    );
    assert_eq!(
        web_events,
        [
            "info:reading app-group",
            "info:reading web-target",
            "info:reading alpha-web",
        ]
    );
    // A target's reads overlap (DIVERGENCES l1): the set per target is fixed,
    // and every api read happens before any web read.
    let sorted = |reads: &[String]| {
        let mut reads = reads.to_vec();
        reads.sort_unstable();
        reads
    };
    let reads = op.reads();
    assert_eq!(reads.len(), 8, "{reads:?}");
    assert_eq!(
        sorted(&reads[..5]),
        ["alpha", "alpha-api", "api-group", "api-target", "shared"]
    );
    assert_eq!(
        sorted(&reads[5..]),
        ["alpha-web", "app-group", "web-target"]
    );
}

#[test]
fn cache_key_is_id_or_title_and_events_use_title_or_id() {
    let matcher = ts_matcher();
    let mut values = HashMap::new();
    values.insert("only-title".to_string(), env(&[("A", "1")]));
    values.insert("id-1".to_string(), env(&[("B", "2")]));
    let op = UnitOp::new(values);
    let mut tags = COMMON.to_vec();
    tags.push("layer:shared");
    let candidates = vec![
        item("", "only-title", &tags),
        item("id-1", "", &tags),
        // Same id as above, different title: served from the cache.
        item("id-1", "Other title", &tags),
    ];
    let mut cache = HashMap::new();
    let (result, events) = merge(&op, &candidates, "api", &matcher, &mut cache);
    let result = result.unwrap();
    assert_eq!(events, ["info:reading only-title", "info:reading id-1"]);
    let mut reads = op.reads();
    reads.sort_unstable();
    assert_eq!(reads, ["id-1", "only-title"]);
    assert_eq!(result.layers.len(), 3);
    assert_eq!(result.layers[2].values, env(&[("B", "2")]));
    assert!(result.overwritten_keys.is_empty());
}

#[test]
fn duplicate_items_are_read_once_and_not_reported_as_overwritten() {
    let op = UnitOp::new(ts_values());
    let shared = tagged("shared", &["layer:shared"]);
    let mut cache = HashMap::new();
    let (result, events) = merge(
        &op,
        &[shared.clone(), shared],
        "api",
        &ts_matcher(),
        &mut cache,
    );
    let result = result.unwrap();
    assert_eq!(events, ["info:reading shared"]);
    assert_eq!(layer_ids(&result.layers), ["shared", "shared"]);
    assert!(result.overwritten_keys.is_empty());
}

#[test]
fn a_failed_read_aborts_after_its_reading_event() {
    let mut op = UnitOp::new(ts_values());
    op.failing.push("api-target".to_string());
    let mut cache = HashMap::new();
    let (result, events) = merge(&op, &ts_items(), "api", &ts_matcher(), &mut cache);
    assert_eq!(result.unwrap_err(), Error::internal("op read failed"));
    assert_eq!(
        events,
        [
            "info:reading shared",
            "info:reading api-group",
            "info:reading api-target",
        ]
    );
    assert!(!cache.contains_key("api-target"));
}

#[test]
fn no_classified_items_merge_to_nothing_without_reads() {
    let op = UnitOp::new(HashMap::new());
    let mut cache = HashMap::new();
    let (result, events) = merge(
        &op,
        &[tagged(
            "beta-web",
            &["layer:client-target", "client:beta", "group:apps"],
        )],
        "web",
        &ts_matcher(),
        &mut cache,
    );
    let result = result.unwrap();
    assert!(result.values.is_empty() && result.layers.is_empty());
    assert!(result.overwritten_keys.is_empty());
    assert!(events.is_empty() && op.reads().is_empty());
}

#[test]
fn overwritten_keys_need_a_different_value_and_sort_by_utf16() {
    let layer = |id: &str, title: &str, pairs: &[(&str, &str)]| EnvLayer {
        name: LayerName::Shared,
        rank: 1,
        item: item(id, title, &[]),
        values: env(pairs),
    };
    let result = merge_layers(vec![
        layer(
            "1",
            "a",
            &[
                ("SAME", "x"),
                ("B", "1"),
                ("\u{1f600}", "1"),
                ("\u{ff21}", "1"),
            ],
        ),
        layer(
            "2",
            "b",
            &[
                ("SAME", "x"),
                ("\u{ff21}", "2"),
                ("\u{1f600}", "2"),
                ("B", "2"),
            ],
        ),
        layer("3", "c", &[("B", "1")]),
    ]);
    // U+1F600 is a surrogate pair (0xD83D ...) and sorts before U+FF21 in
    // UTF-16, although its code point is larger.
    assert_eq!(result.overwritten_keys, ["B", "\u{1f600}", "\u{ff21}"]);
    assert_eq!(result.values["B"], "1");
    assert_eq!(
        result.values.keys().collect::<Vec<_>>(),
        ["SAME", "B", "\u{1f600}", "\u{ff21}"]
    );
}

// ---------------------------------------------------------------------------
// Collation (D8; plan risk 1)
// ---------------------------------------------------------------------------

#[test]
fn collation_matches_the_oracle_for_every_golden_pair() {
    let mut group_of = vec![usize::MAX; TITLES.len()];
    for (group, members) in GROUPS.iter().enumerate() {
        for &index in *members {
            assert_eq!(group_of[index], usize::MAX, "title {index} in two groups");
            group_of[index] = group;
        }
    }
    assert!(
        group_of.iter().all(|&group| group != usize::MAX),
        "every title is in a group"
    );

    let mut mismatches = Vec::new();
    for (left, left_title) in TITLES.iter().enumerate() {
        for (right, right_title) in TITLES.iter().enumerate() {
            let expected = group_of[left].cmp(&group_of[right]);
            let actual = compare_titles(left_title, right_title);
            if actual != expected {
                mismatches.push(format!(
                    "{left_title:?} vs {right_title:?}: oracle {expected:?}, rust {actual:?}"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn collation_golden_merge_picks_the_oracle_winner() {
    let ids: Vec<String> = (0..TITLES.len())
        .map(|index| format!("t{index:03}"))
        .collect();
    let pairs: Vec<(&str, &str)> = ids
        .iter()
        .zip(TITLES)
        .map(|(id, title)| (id.as_str(), *title))
        .collect();
    let result = merge_layers(shared_layers(&pairs));

    // Oracle layer order: groups ascending, API order inside a group.
    let expected: Vec<String> = GROUPS
        .iter()
        .flat_map(|group| {
            let mut members = group.to_vec();
            members.sort_unstable();
            members.into_iter().map(|index| ids[index].clone())
        })
        .collect();
    assert_eq!(layer_ids(&result.layers), expected);
    assert_eq!(format!("COMMON={}\n", result.values["COMMON"]), FULL_ENV);
}

#[test]
fn collation_pair_winners_match_the_oracle() {
    for &(left, right, left_first, right_first) in PAIRS {
        let winner =
            |layers: &[(&str, &str)]| merge_layers(shared_layers(layers)).values["COMMON"].clone();
        assert_eq!(
            winner(&[("L", left), ("R", right)]),
            left_first,
            "{left:?} then {right:?}"
        );
        assert_eq!(
            winner(&[("R", right), ("L", left)]),
            right_first,
            "{right:?} then {left:?}"
        );
    }
}

#[test]
fn equal_titles_keep_api_order_without_an_id_tie_break() {
    let result = merge_layers(shared_layers(&[
        ("z-id", "dup"),
        ("a-id", "dup"),
        ("m-id", "dup"),
    ]));
    assert_eq!(layer_ids(&result.layers), ["z-id", "a-id", "m-id"]);
    assert_eq!(result.values["COMMON"], "m-id");
}

#[test]
fn rank_orders_before_title() {
    let mut layers = shared_layers(&[("client", "a")]);
    layers[0].name = LayerName::Client;
    layers[0].rank = LayerName::Client.rank();
    layers.extend(shared_layers(&[("shared", "z")]));
    let result = merge_layers(layers);
    assert_eq!(layer_ids(&result.layers), ["shared", "client"]);
    assert_eq!(result.values["COMMON"], "client");
}

/// **Known divergence, approved under DIVERGENCES.md row g1.**
///
/// ICU4C 78 (Bun 1.4.0 and Node 24) orders Han ideographs by radical and
/// stroke; ICU4X 2.4 `en-US` orders them by code point (UCA implicit
/// weights). Oracle values from `bun -e` with `LANG=en_US.UTF-8`:
///
/// ```text
/// "密钥".localeCompare("密鑰") === -1   // U+5BC6 U+94A5 vs U+5BC6 U+9470
/// "丗".localeCompare("且") === -1       // U+4E17 vs U+4E14
/// "㐀".localeCompare("丞") === -1       // U+3400 (Ext. A) vs U+4E1E
/// ```
///
/// This test pins the current Rust behavior so the divergence stays visible.
#[test]
fn known_divergence_han_ideographs_use_code_point_order() {
    let oracle_less = [
        ("\u{5bc6}\u{94a5}", "\u{5bc6}\u{9470}"),
        ("\u{4e17}", "\u{4e14}"),
        ("\u{3400}", "\u{4e1e}"),
    ];
    for (left, right) in oracle_less {
        assert_eq!(
            compare_titles(left, right),
            Ordering::Greater,
            "{left} vs {right}"
        );
    }
}

/// **Known divergence, approved under DIVERGENCES.md row g1** (Unicode 17+
/// characters follow ICU4X data). U+1AD4 and U+1AD7 are new in Unicode 17:
/// Bun's ICU 78.1 still orders them as unassigned code points, while ICU4X
/// data treats both as ignorable combining marks, so the titles tie and API
/// order decides. Oracle (`bun -e`, `LANG=en_US.UTF-8`):
///
/// ```text
/// "env-a᫔".localeCompare("env-a᫗") === -1   // `R` wins in both orders
/// ```
#[test]
fn known_divergence_unicode_17_marks_tie_in_icu4x() {
    let left = "env-a\u{1ad4}";
    let right = "env-a\u{1ad7}";
    assert_eq!(compare_titles(left, right), Ordering::Equal);
    let winner =
        |layers: &[(&str, &str)]| merge_layers(shared_layers(layers)).values["COMMON"].clone();
    // Oracle: "R" for both API orders. Rust: the later item in API order.
    assert_eq!(winner(&[("L", left), ("R", right)]), "R");
    assert_eq!(winner(&[("R", right), ("L", left)]), "L");
}

// ---------------------------------------------------------------------------
// Overlapped reads (DIVERGENCES l1)
// ---------------------------------------------------------------------------

/// Env reads that hold until `gate` reads have started, tracking the peak
/// number in flight. Per key: an optional delay, then a failure message or a
/// panic.
struct GateOp {
    gate: usize,
    arrived: Mutex<usize>,
    cv: Condvar,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    timed_out: AtomicBool,
    delays: HashMap<String, u64>,
    failing: HashMap<String, String>,
    panicking: Vec<String>,
    reads: Mutex<Vec<String>>,
}

impl GateOp {
    fn new(gate: usize) -> Self {
        GateOp {
            gate,
            arrived: Mutex::new(0),
            cv: Condvar::new(),
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            timed_out: AtomicBool::new(false),
            delays: HashMap::new(),
            failing: HashMap::new(),
            panicking: Vec::new(),
            reads: Mutex::new(Vec::new()),
        }
    }

    fn delay(mut self, key: &str, millis: u64) -> Self {
        self.delays.insert(key.to_string(), millis);
        self
    }

    fn fail(mut self, key: &str, message: &str) -> Self {
        self.failing.insert(key.to_string(), message.to_string());
        self
    }

    fn panic_on(mut self, key: &str) -> Self {
        self.panicking.push(key.to_string());
        self
    }
}

impl OnePassword for GateOp {
    fn assert_authenticated(&self) -> Result<()> {
        unreachable!("reads never authenticate")
    }

    fn list_items_by_tags(&self, _tags: &[String]) -> Result<Vec<ItemSummary>> {
        unreachable!("reads never list")
    }

    fn read_config_item(&self, _item: &str) -> Result<String> {
        unreachable!("reads never read config items")
    }

    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap> {
        let key = item.key();
        self.reads.lock().unwrap().push(key.to_string());
        let now = self.in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        self.peak.fetch_max(now, AtomicOrdering::SeqCst);
        {
            let mut arrived = self.arrived.lock().unwrap();
            *arrived += 1;
            self.cv.notify_all();
            let (arrived, _) = self
                .cv
                .wait_timeout_while(arrived, Duration::from_secs(10), |count| *count < self.gate)
                .unwrap();
            if *arrived < self.gate {
                self.timed_out.store(true, AtomicOrdering::SeqCst);
            }
        }
        if let Some(millis) = self.delays.get(key) {
            std::thread::sleep(Duration::from_millis(*millis));
        }
        self.in_flight.fetch_sub(1, AtomicOrdering::SeqCst);
        if self.panicking.iter().any(|k| k == key) {
            panic!("read panicked: {key}");
        }
        match self.failing.get(key) {
            Some(message) => Err(Error::internal(message.clone())),
            None => Ok(env(&[("KEY", key)])),
        }
    }
}

fn numbered_items(count: usize) -> Vec<ItemSummary> {
    (0..count)
        .map(|index| item(&format!("i{index}"), "", &[]))
        .collect()
}

/// `read_in_order` over `items`, returning the result and the `on_read` keys.
fn read_all(op: &GateOp, items: &[ItemSummary]) -> (Result<Vec<EnvMap>>, Vec<String>) {
    let refs: Vec<&ItemSummary> = items.iter().collect();
    let mut started = Vec::new();
    let result = read_in_order(op, &refs, &mut |item| started.push(item.key().to_string()));
    (result, started)
}

#[test]
fn reads_overlap_up_to_the_cap_and_return_values_in_order() {
    // Every read holds until READ_CONCURRENCY reads have started, so a
    // sequential implementation times out and a wider one exceeds the peak.
    let items = numbered_items(READ_CONCURRENCY + 3);
    let op = GateOp::new(READ_CONCURRENCY);
    let (result, started) = read_all(&op, &items);
    assert!(!op.timed_out.load(AtomicOrdering::SeqCst));
    assert_eq!(op.peak.load(AtomicOrdering::SeqCst), READ_CONCURRENCY);
    let keys: Vec<String> = items.iter().map(|item| item.key().to_string()).collect();
    assert_eq!(started, keys);
    assert_eq!(
        result.unwrap(),
        keys.iter()
            .map(|key| env(&[("KEY", key)]))
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_first_failure_in_order_wins_and_stops_on_read() {
    for _ in 0..50 {
        let op = GateOp::new(1).fail("i2", "two").fail("i5", "five");
        let (result, started) = read_all(&op, &numbered_items(12));
        assert_eq!(result, Err(Error::internal("two")));
        assert_eq!(started, ["i0", "i1", "i2"]);
    }
}

#[test]
fn a_slow_earlier_failure_beats_a_fast_later_one() {
    // Completion order is reversed: i1 fails first, but i0 is earlier.
    let op = GateOp::new(2)
        .delay("i0", 150)
        .fail("i0", "zero")
        .fail("i1", "one");
    let (result, started) = read_all(&op, &numbered_items(4));
    assert_eq!(result, Err(Error::internal("zero")));
    assert_eq!(started, ["i0"]);

    let op = GateOp::new(2).delay("i0", 150).fail("i1", "one");
    let (result, started) = read_all(&op, &numbered_items(4));
    assert_eq!(result, Err(Error::internal("one")));
    assert_eq!(started, ["i0", "i1"]);
}

#[test]
fn a_later_panic_never_replaces_an_earlier_error() {
    let op = GateOp::new(2)
        .delay("i0", 100)
        .fail("i0", "zero")
        .panic_on("i1");
    let (result, _) = read_all(&op, &numbered_items(3));
    assert_eq!(result, Err(Error::internal("zero")));
}

#[test]
fn a_panic_reached_in_order_is_resumed() {
    let panic = std::panic::catch_unwind(|| {
        let op = GateOp::new(2).panic_on("i0");
        read_all(&op, &numbered_items(3))
    })
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<String>().unwrap(), "read panicked: i0");
}

#[test]
fn a_single_read_stays_on_the_calling_thread() {
    let items = numbered_items(1);
    let op = GateOp::new(1);
    let caller = std::thread::current().id();
    struct ThreadCheck<'a>(&'a GateOp, std::thread::ThreadId);
    impl OnePassword for ThreadCheck<'_> {
        fn assert_authenticated(&self) -> Result<()> {
            unreachable!()
        }
        fn list_items_by_tags(&self, _tags: &[String]) -> Result<Vec<ItemSummary>> {
            unreachable!()
        }
        fn read_config_item(&self, _item: &str) -> Result<String> {
            unreachable!()
        }
        fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap> {
            assert_eq!(std::thread::current().id(), self.1);
            self.0.read_env_item(item)
        }
    }
    let check = ThreadCheck(&op, caller);
    let result = read_in_order(&check, &[&items[0]], &mut |_| {});
    assert_eq!(result.unwrap(), [env(&[("KEY", "i0")])]);
}
