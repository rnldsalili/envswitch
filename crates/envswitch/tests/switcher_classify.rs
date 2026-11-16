//! `classify_item_for_target` through the public API (contract 02 §8.2 rows
//! 10–11, §3.5 erratum, §4.2). Owner: C3.
//!
//! Expected values come from the TS oracle (`src/switcher.ts` at `53de029`,
//! Bun 1.4.0) via `classifyItemForTarget` with the same configs and items
//! (`src/switcher/oracle/scenarios.ts` prints them).
//! These tests need the real config module (C1): they are ignored until C1
//! merges, then run at the C3 merge gate.

mod support;

use std::path::{Path, PathBuf};

use envswitch::config::parse_config;
use envswitch::model::{ClassifyParams, Config, LayerName};
use envswitch::switcher::classify_item_for_target;
use serde_json::json;

use support::item;

fn config(value: serde_json::Value) -> Config {
    parse_config(&value.to_string()).expect("valid config")
}

/// The config of `switcher.test.ts`.
fn ts_config() -> Config {
    config(json!({
        "project": { "id": "demo" },
        "clients": ["alpha", "beta"],
        "environments": ["local"],
        "targets": [
            { "name": "api", "output": "apps/api/.env", "groups": ["apis"] },
            { "name": "web", "output": "apps/web/.env", "groups": ["apps"] }
        ],
        "tags": { "base": ["envswitch"] }
    }))
}

fn cwd() -> PathBuf {
    PathBuf::from("/tmp/envswitch-cwd/demo-suite")
}

fn params(target: &str) -> ClassifyParams {
    ClassifyParams {
        env: "local".to_string(),
        client: "alpha".to_string(),
        target: target.to_string(),
    }
}

const COMMON: [&str; 3] = ["envswitch", "project:demo", "env:local"];

fn classify(config: &Config, extra: &[&str], target: &str) -> Option<LayerName> {
    let mut tags = COMMON.to_vec();
    tags.extend_from_slice(extra);
    classify_item_for_target(config, &item("x", "x", &tags), &params(target), &cwd())
}

#[test]
#[ignore = "needs C1/C2"]
fn classifies_only_items_that_belong_to_the_selected_target() {
    let config = ts_config();
    let cases: [(&[&str], &str, Option<LayerName>); 18] = [
        (&["layer:shared"], "api", Some(LayerName::Shared)),
        (
            &["layer:group", "group:apis"],
            "api",
            Some(LayerName::Group),
        ),
        (&["layer:group", "group:apps"], "api", None),
        (
            &["layer:group", "group:apps"],
            "web",
            Some(LayerName::Group),
        ),
        (&["layer:shared", "group:apis"], "api", None),
        (
            &["layer:target", "target:api"],
            "api",
            Some(LayerName::Target),
        ),
        (&["layer:target", "target:web"], "api", None),
        (&["layer:target", "target:api", "group:apis"], "api", None),
        (&["layer:target", "target:api", "client:alpha"], "api", None),
        (
            &["layer:client", "client:alpha"],
            "api",
            Some(LayerName::Client),
        ),
        (
            &["layer:client-target", "client:alpha", "group:apps"],
            "api",
            None,
        ),
        (
            &["layer:client-target", "client:alpha", "group:apps"],
            "web",
            Some(LayerName::ClientTarget),
        ),
        (
            &["layer:client-target", "client:alpha", "group:apis"],
            "api",
            Some(LayerName::ClientTarget),
        ),
        (
            &["layer:client-target", "client:beta", "group:apps"],
            "web",
            None,
        ),
        (
            &["layer:client-target", "client:alpha", "target:web"],
            "web",
            None,
        ),
        (&["layer:shared", "client:gamma"], "api", None),
        (&["layer:shared", "target:worker"], "api", None),
        (&["layer:shared", "group:workers"], "api", None),
    ];
    for (extra, target, expected) in cases {
        assert_eq!(
            classify(&config, extra, target),
            expected,
            "{extra:?} for {target}"
        );
    }
}

#[test]
#[ignore = "needs C1/C2"]
fn recognizes_unknown_scopes_with_custom_tag_templates() {
    let config = config(json!({
        "project": { "id": "demo" },
        "clients": ["alpha"],
        "targets": [{ "name": "api", "output": ".env", "groups": ["apis"] }],
        "tags": {
            "base": ["envswitch"],
            "client": "scope/client/{client}",
            "target": "scope/target/{target}",
            "group": "scope/group/{group}"
        }
    }));
    for tag in [
        "scope/client/beta",
        "scope/target/worker",
        "scope/group/workers",
    ] {
        assert_eq!(
            classify(&config, &["layer:shared", tag], "api"),
            None,
            "{tag}"
        );
    }
}

#[test]
#[ignore = "needs C1/C2"]
fn scope_wildcard_excludes_exactly_the_js_line_terminators() {
    // Oracle table (`classifyItemForTarget` on a `layer:shared` item with one
    // extra tag): CR, LF, U+2028, U+2029 never match the scope wildcard, NEL
    // and TAB do, and the wildcard needs at least one character.
    let config = config(json!({
        "project": { "id": "demo" },
        "targets": [{ "name": "api", "output": ".env", "groups": ["apis"] }],
        "tags": { "base": ["envswitch"] }
    }));
    for scope in ["client", "target", "group"] {
        for terminator in ["\r", "\n", "\u{2028}", "\u{2029}"] {
            for tag in [
                format!("{scope}:foreign{terminator}"),
                format!("{scope}:{terminator}"),
                format!("{scope}:a{terminator}b"),
                format!("{terminator}{scope}:foreign"),
            ] {
                assert_eq!(
                    classify(&config, &["layer:shared", &tag], "api"),
                    Some(LayerName::Shared),
                    "{tag:?}"
                );
            }
        }
        for other in ["\u{85}", "\t", ""] {
            for tag in [
                format!("{scope}:foreign{other}"),
                format!("{scope}:a{other}b"),
            ] {
                assert_eq!(
                    classify(&config, &["layer:shared", &tag], "api"),
                    None,
                    "{tag:?}"
                );
            }
            let leading = format!("{other}{scope}:foreign");
            let expected = if other.is_empty() {
                None
            } else {
                Some(LayerName::Shared)
            };
            assert_eq!(
                classify(&config, &["layer:shared", &leading], "api"),
                expected,
                "{leading:?}"
            );
        }
        let bare = format!("{scope}:");
        assert_eq!(
            classify(&config, &["layer:shared", &bare], "api"),
            Some(LayerName::Shared)
        );
        for other in ["\u{85}", "\t"] {
            let tag = format!("{scope}:{other}");
            assert_eq!(
                classify(&config, &["layer:shared", &tag], "api"),
                None,
                "{tag:?}"
            );
        }
    }
}

/// `(case, tags overrides, extra item tags, oracle result)`.
type TemplateCase = (
    &'static str,
    serde_json::Value,
    &'static [&'static str],
    Option<LayerName>,
);

#[test]
#[ignore = "needs C1/C2"]
fn custom_template_cases_match_the_oracle() {
    use LayerName::{Client, Group, Shared};
    let cases: Vec<TemplateCase> = vec![
        // Regex metacharacters are literal; `{2}` is an (unknown) placeholder
        // and renders "", so the target tag is `t[api]`.
        (
            "metachars",
            json!({ "client": "c.{client}+(x)", "target": "t[{target}]{2}", "group": "g^{group}$|*?\\" }),
            &["layer:shared"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "c.beta+(x)"],
            None,
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "cZbeta+(x)"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "c.beta+(x)x"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "c.+(x)"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "t[worker]{2}"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "t[worker]]"],
            None,
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "tworker"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "g^workers$|*?\\"],
            None,
        ),
        (
            "metachars",
            json!(null),
            &["layer:shared", "gworkers"],
            Some(Shared),
        ),
        (
            "metachars",
            json!(null),
            &["layer:client", "c.alpha+(x)"],
            Some(Client),
        ),
        (
            "metachars",
            json!(null),
            &["layer:target", "t[api]{2}"],
            None,
        ),
        (
            "metachars",
            json!(null),
            &["layer:group", "g^apis$|*?\\"],
            Some(Group),
        ),
        (
            "multi-sentinel",
            json!({ "client": "{client}/{client}" }),
            &["layer:shared", "a/b"],
            None,
        ),
        (
            "multi-sentinel",
            json!(null),
            &["layer:shared", "a/"],
            Some(Shared),
        ),
        (
            "multi-sentinel",
            json!(null),
            &["layer:shared", "/b"],
            Some(Shared),
        ),
        (
            "multi-sentinel",
            json!(null),
            &["layer:shared", "a/b/c"],
            None,
        ),
        (
            "multi-sentinel",
            json!(null),
            &["layer:client", "alpha/alpha"],
            Some(Client),
        ),
        (
            "no-placeholder",
            json!({ "client": "client" }),
            &["layer:shared", "client"],
            None,
        ),
        (
            "no-placeholder",
            json!(null),
            &["layer:shared", "client:alpha"],
            Some(Shared),
        ),
        (
            "no-placeholder",
            json!(null),
            &["layer:client", "client"],
            Some(Client),
        ),
        (
            "bare",
            json!({ "client": "{client}" }),
            &["layer:shared"],
            None,
        ),
        ("bare", json!(null), &["layer:target", "target:api"], None),
        (
            "bare",
            json!(null),
            &["layer:client", "alpha"],
            Some(Client),
        ),
        (
            "bare",
            json!(null),
            &["layer:client", "alpha", "target:api"],
            None,
        ),
        (
            "env-only-client",
            json!({ "client": "{env}" }),
            &["layer:shared"],
            Some(Shared),
        ),
        ("env-only-client", json!(null), &["layer:shared", ""], None),
        ("env-only-client", json!(null), &["layer:client", ""], None),
        (
            "env-only-client",
            json!(null),
            &["layer:client", "local"],
            None,
        ),
        (
            "empty-layer",
            json!({ "layer": "{env}" }),
            &["layer:shared"],
            None,
        ),
        ("empty-layer", json!(null), &[""], None),
        ("empty-layer", json!(null), &["local"], None),
        (
            "project-in-scope",
            json!({ "client": "{project}/{client}" }),
            &["layer:shared", "demo/beta"],
            None,
        ),
        (
            "project-in-scope",
            json!(null),
            &["layer:shared", "other/beta"],
            Some(Shared),
        ),
        (
            "project-in-scope",
            json!(null),
            &["layer:client", "demo/alpha"],
            Some(Client),
        ),
    ];

    let mut current: Option<Config> = None;
    for (name, overrides, extra, expected) in cases {
        if !overrides.is_null() {
            let mut tags = json!({ "base": ["envswitch"] });
            for (key, value) in overrides.as_object().unwrap() {
                tags[key] = value.clone();
            }
            current = Some(config(json!({
                "project": { "id": "demo" },
                "targets": [{ "name": "api", "output": ".env", "groups": ["apis"] }],
                "tags": tags
            })));
        }
        let config = current.as_ref().unwrap();
        assert_eq!(classify(config, extra, "api"), expected, "{name} {extra:?}");
    }
}

#[test]
#[ignore = "needs C1/C2"]
fn project_placeholder_falls_back_to_the_cwd_basename() {
    // Oracle run with `process.chdir(<tmp>/demo-suite)`.
    let config = config(json!({
        "targets": [{ "name": "api", "output": ".env" }],
        "tags": { "base": ["app:{project}"] }
    }));
    let classify_tags = |tags: &[&str], cwd: &Path| {
        classify_item_for_target(&config, &item("x", "x", tags), &params("api"), cwd)
    };
    assert_eq!(
        classify_tags(&["app:demo-suite", "env:local", "layer:shared"], &cwd()),
        Some(LayerName::Shared)
    );
    assert_eq!(
        classify_tags(&["app:other", "env:local", "layer:shared"], &cwd()),
        None
    );
    assert_eq!(classify_tags(&["env:local", "layer:shared"], &cwd()), None);
    // The `cwd` argument, not the process cwd, drives the fallback.
    assert_eq!(
        classify_tags(
            &["app:other", "env:local", "layer:shared"],
            Path::new("/somewhere/other")
        ),
        Some(LayerName::Shared)
    );
}
