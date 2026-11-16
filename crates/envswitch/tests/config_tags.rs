//! Tag templates (contract 02 §3). Expectations were confirmed against the
//! TS regex replace and `tagValues` / `candidateListTags` under Bun 1.4.0,
//! except the JS prototype placeholders, which render `""` (DIVERGENCES a1).

use std::path::Path;

use envswitch::config::tags::{
    DEFAULT_CLIENT_TEMPLATE, DEFAULT_ENV_TEMPLATE, DEFAULT_GROUP_TEMPLATE, DEFAULT_LAYER_TEMPLATE,
    DEFAULT_PROJECT_TEMPLATE, DEFAULT_TARGET_TEMPLATE,
};
use envswitch::config::{
    TemplateContext, candidate_list_tags, compact_unique, parse_config, render_template, tag_values,
};
use envswitch::model::{Config, LayerName, TagParams, TagValues};
use serde_json::{Value, json};

fn config(extra: Value) -> Config {
    let mut raw = json!({ "targets": [{ "name": "api", "output": "apps/api/.env" }] });
    for (key, value) in extra.as_object().expect("object") {
        raw[key] = value.clone();
    }
    parse_config(&raw.to_string()).unwrap()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

const CWD: &str = "/work/cwd-project";

// --- render_template (02 §3.1) ---------------------------------------------

#[test]
fn render_template_matches_the_ts_regex_replace() {
    let ctx = TemplateContext {
        project: "demo",
        env: "local",
        client: "{client}",
        target: "api",
        group: "apis",
        layer: "client-target",
    };
    let cases = [
        ("env:{env}", "env:local"),
        (
            "{project}/{env}/{client}/{target}/{group}/{layer}",
            "demo/local/{client}/api/apis/client-target",
        ),
        ("{{env}}", "{local}"),
        ("{ env }", "{ env }"),
        ("{}", "{}"),
        ("{env", "{env"),
        ("env}", "env}"),
        ("{unknown}", ""),
        ("{a{env}", "{alocal"),
        ("{env}{env}", "locallocal"),
        ("{ENV}", ""),
        ("{env-x}", ""),
        ("{env.x}", "{env.x}"),
        ("{\u{e9}}", "{\u{e9}}"),
        ("{_}", ""),
        ("{0}", ""),
        ("$&{env}$1", "$&local$1"),
        ("", ""),
        ("}{env}{", "}local{"),
        ("{{{env}}}", "{{local}}"),
        ("{env}}", "local}"),
        ("{-}", ""),
        ("caf\u{e9}:{env}:\u{1f600}", "caf\u{e9}:local:\u{1f600}"),
    ];
    for (template, expected) in cases {
        assert_eq!(render_template(template, &ctx), expected, "{template:?}");
    }
}

#[test]
fn substituted_values_are_not_rescanned() {
    let ctx = TemplateContext {
        env: "{client}",
        client: "alpha",
        ..TemplateContext::default()
    };
    assert_eq!(render_template("{env}-{client}", &ctx), "{client}-alpha");
}

#[test]
fn js_prototype_placeholders_render_empty() {
    // DIVERGENCES a1: TS renders JS internals (`function Object() ...`).
    let ctx = TemplateContext {
        env: "local",
        ..TemplateContext::default()
    };
    for name in [
        "constructor",
        "__proto__",
        "toString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "toLocaleString",
        "__defineGetter__",
        "__lookupSetter__",
    ] {
        assert_eq!(
            render_template(&format!("x:{{{name}}}:{{env}}"), &ctx),
            "x::local",
            "{name}"
        );
    }
}

// --- tag_values (02 §3.2) --------------------------------------------------

#[test]
fn default_templates() {
    assert_eq!(DEFAULT_PROJECT_TEMPLATE, "project:{project}");
    assert_eq!(DEFAULT_ENV_TEMPLATE, "env:{env}");
    assert_eq!(DEFAULT_CLIENT_TEMPLATE, "client:{client}");
    assert_eq!(DEFAULT_TARGET_TEMPLATE, "target:{target}");
    assert_eq!(DEFAULT_GROUP_TEMPLATE, "group:{group}");
    assert_eq!(DEFAULT_LAYER_TEMPLATE, "layer:{layer}");
}

#[test]
fn tag_values_with_defaults() {
    let config = config(json!({ "project": { "id": "demo" } }));
    let params = TagParams {
        env: Some("local"),
        client: Some("alpha"),
        target: Some("api"),
        group: Some("apis"),
        layer: Some(LayerName::ClientTarget),
    };
    assert_eq!(
        tag_values(&config, &params, Path::new(CWD)),
        TagValues {
            base: Vec::new(),
            project: Some("project:demo".to_string()),
            env: Some("env:local".to_string()),
            client: Some("client:alpha".to_string()),
            target: Some("target:api".to_string()),
            group: Some("group:apis".to_string()),
            layer: Some("layer:client-target".to_string()),
        }
    );
}

#[test]
fn tag_values_renders_only_passed_params() {
    let config = config(json!({ "project": { "id": "demo" } }));
    let params = TagParams {
        env: Some("local"),
        client: Some(""),
        ..TagParams::default()
    };
    assert_eq!(
        tag_values(&config, &params, Path::new(CWD)),
        TagValues {
            base: Vec::new(),
            project: Some("project:demo".to_string()),
            env: Some("env:local".to_string()),
            ..TagValues::default()
        }
    );
    let nothing = tag_values(&config, &TagParams::default(), Path::new(CWD));
    assert_eq!(nothing.env, None);
    assert_eq!(nothing.layer, None);
}

#[test]
fn custom_templates_override_only_their_keys() {
    let config = config(json!({
        "project": { "id": "demo" },
        "tags": {
            "base": ["envswitch", "{project}-{env}", "{client}", "envswitch", "{unknown}"],
            "env": "scope/{env}/{client}",
            "client": "{client}",
            "layer": "L={layer}",
        },
    }));
    let params = TagParams {
        env: Some("local"),
        layer: Some(LayerName::Shared),
        ..TagParams::default()
    };
    let values = tag_values(&config, &params, Path::new(CWD));
    // Base: empty renders dropped, order and duplicates kept.
    assert_eq!(
        values.base,
        strings(&["envswitch", "demo-local", "envswitch"])
    );
    assert_eq!(values.project.as_deref(), Some("project:demo"));
    // A rendered tag can be empty or partial when its other placeholders
    // were not passed.
    assert_eq!(values.env.as_deref(), Some("scope/local/"));
    assert_eq!(values.client, None);
    assert_eq!(values.layer.as_deref(), Some("L=shared"));

    let client_only = TagParams {
        env: Some("local"),
        client: Some("alpha"),
        ..TagParams::default()
    };
    let values = tag_values(&config, &client_only, Path::new(CWD));
    assert_eq!(
        values.base,
        strings(&["envswitch", "demo-local", "alpha", "envswitch"])
    );
    assert_eq!(values.client.as_deref(), Some("alpha"));
}

#[test]
fn project_placeholder_falls_back_to_the_cwd_basename() {
    let params = TagParams {
        env: Some("local"),
        ..TagParams::default()
    };
    let base = json!({ "tags": { "base": ["p={project}"] } });

    // No project: basename of the cwd, and no project tag.
    let values = tag_values(&config(base.clone()), &params, Path::new(CWD));
    assert_eq!(values.base, strings(&["p=cwd-project"]));
    assert_eq!(values.project, None);

    // Project without id: same fallback.
    let mut with_project = base.clone();
    with_project["project"] = json!({});
    let values = tag_values(&config(with_project), &params, Path::new(CWD));
    assert_eq!(values.base, strings(&["p=cwd-project"]));
    assert_eq!(values.project, None);

    // `id: ""` is kept (`??`), renders empty, and still yields no project tag.
    let mut empty_id = base.clone();
    empty_id["project"] = json!({ "id": "" });
    let values = tag_values(&config(empty_id), &params, Path::new(CWD));
    assert_eq!(values.base, strings(&["p="]));
    assert_eq!(values.project, None);

    // A cwd at the filesystem root has an empty basename.
    let values = tag_values(&config(base.clone()), &params, Path::new("/"));
    assert_eq!(values.base, strings(&["p="]));
    // Trailing separators are ignored, like Node `path.basename`.
    let values = tag_values(&config(base), &params, Path::new("/work/cwd-project/"));
    assert_eq!(values.base, strings(&["p=cwd-project"]));
}

#[test]
fn layer_names_render_with_their_ts_strings() {
    let config = config(json!({}));
    let rendered: Vec<String> = LayerName::ALL
        .iter()
        .map(|&layer| {
            let params = TagParams {
                layer: Some(layer),
                ..TagParams::default()
            };
            tag_values(&config, &params, Path::new(CWD)).layer.unwrap()
        })
        .collect();
    assert_eq!(
        rendered,
        [
            "layer:shared",
            "layer:group",
            "layer:target",
            "layer:client",
            "layer:client-target"
        ]
    );
}

// --- candidate_list_tags (02 §3.4) -----------------------------------------

#[test]
fn candidate_list_tags_for_the_test_config() {
    let config = config(json!({
        "project": { "id": "demo" },
        "tags": { "base": ["envswitch"] },
    }));
    assert_eq!(
        candidate_list_tags(&config, "local", Path::new(CWD)),
        strings(&["envswitch", "project:demo", "env:local"])
    );
}

#[test]
fn candidate_list_tags_dedupes_and_drops_empties() {
    let config = config(json!({
        "tags": { "base": ["env:{env}", "{client}", "shared", "shared"], "env": "env:{env}" },
    }));
    assert_eq!(
        candidate_list_tags(&config, "local", Path::new(CWD)),
        strings(&["env:local", "shared"])
    );
    // No project tag without a project id; an empty env drops the env tag.
    let config = config_without_tags();
    assert_eq!(
        candidate_list_tags(&config, "", Path::new(CWD)),
        Vec::<String>::new()
    );
    assert_eq!(
        candidate_list_tags(&config, "dev", Path::new(CWD)),
        strings(&["env:dev"])
    );
}

fn config_without_tags() -> Config {
    config(json!({}))
}

// --- compact_unique (02 §3.3) ----------------------------------------------

#[test]
fn compact_unique_keeps_first_occurrences_in_order() {
    let values = [
        Some("b"),
        None,
        Some("a"),
        Some(""),
        Some("b"),
        Some("c"),
        Some("a"),
    ];
    assert_eq!(compact_unique(values), strings(&["b", "a", "c"]));
    assert_eq!(
        compact_unique(Vec::<Option<String>>::new()),
        Vec::<String>::new()
    );
    let owned = vec![
        Some("x".to_string()),
        Some(String::new()),
        None,
        Some("x".to_string()),
    ];
    assert_eq!(compact_unique(owned), strings(&["x"]));
}
