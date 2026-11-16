//! Config parsing and selection (contract 02 §1, §8.2 rows 1–4 and 8).
//! Every message is asserted verbatim; the oracle is `src/config.ts`.

use envswitch::Error;
use envswitch::config::{parse_config, select_targets, validate_selection};
use envswitch::model::{Config, ProjectConfig, RootMarker, TagTemplates, TargetConfig};
use serde_json::{Value, json};

/// `parse_config(value.to_string())`.
fn parse(value: Value) -> envswitch::Result<Config> {
    parse_config(&value.to_string())
}

/// The `Error::User` message of a failed parse.
fn parse_error(raw: &str) -> String {
    match parse_config(raw) {
        Ok(config) => panic!("expected an error for {raw}, got {config:?}"),
        Err(Error::User(message)) => message,
        Err(other) => panic!("expected a user error for {raw}, got {other:?}"),
    }
}

/// A config with one valid target plus `extra` merged into the root object.
fn with_target(extra: Value) -> Value {
    let mut config = json!({ "targets": [{ "name": "api", "output": "apps/api/.env" }] });
    for (key, value) in extra.as_object().expect("object") {
        config[key] = value.clone();
    }
    config
}

fn target(name: &str, output: &str) -> TargetConfig {
    TargetConfig {
        name: name.to_string(),
        output: output.to_string(),
        groups: None,
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

// --- 02 §8.2 row 1 ---------------------------------------------------------

#[test]
fn parses_a_valid_generic_config() {
    let config = parse(json!({
        "project": { "id": "demo" },
        "clients": ["alpha"],
        "environments": ["local"],
        "targets": [{ "name": "api", "output": "apps/api/.env", "groups": ["apis"] }],
        "tags": { "base": ["envswitch"], "group": "group:{group}" },
    }))
    .unwrap();

    assert_eq!(config.project.as_ref().unwrap().id.as_deref(), Some("demo"));
    assert_eq!(config.targets[0].name, "api");
    assert_eq!(config.targets[0].groups, Some(strings(&["apis"])));
    assert_eq!(
        config.tags.as_ref().unwrap().group.as_deref(),
        Some("group:{group}")
    );
    assert_eq!(
        config,
        Config {
            project: Some(ProjectConfig {
                id: Some("demo".to_string()),
                root_markers: None,
            }),
            clients: Some(strings(&["alpha"])),
            environments: Some(strings(&["local"])),
            targets: vec![TargetConfig {
                groups: Some(strings(&["apis"])),
                ..target("api", "apps/api/.env")
            }],
            tags: Some(TagTemplates {
                base: Some(strings(&["envswitch"])),
                group: Some("group:{group}".to_string()),
                ..TagTemplates::default()
            }),
        }
    );
}

#[test]
fn minimal_config_leaves_optional_fields_absent() {
    let config = parse(json!({ "targets": [{ "name": "api", "output": "x" }] })).unwrap();
    assert_eq!(
        config,
        Config {
            project: None,
            clients: None,
            environments: None,
            targets: vec![target("api", "x")],
            tags: None,
        }
    );
}

#[test]
fn keeps_empty_collections_and_empty_project_id_distinct_from_absent() {
    let config = parse(with_target(json!({
        "project": { "id": "", "rootMarkers": [] },
        "clients": [],
        "environments": [],
        "tags": { "base": [] },
    })))
    .unwrap();
    assert_eq!(
        config.project,
        Some(ProjectConfig {
            id: Some(String::new()),
            root_markers: Some(Vec::new()),
        })
    );
    assert_eq!(config.clients, Some(Vec::new()));
    assert_eq!(config.environments, Some(Vec::new()));
    assert_eq!(
        config.tags,
        Some(TagTemplates {
            base: Some(Vec::new()),
            ..TagTemplates::default()
        })
    );

    let config = parse(
        json!({ "targets": [{ "name": "api", "output": "x", "groups": [] }], "project": {} }),
    )
    .unwrap();
    assert_eq!(config.targets[0].groups, Some(Vec::new()));
    assert_eq!(config.project, Some(ProjectConfig::default()));
}

#[test]
fn stores_values_untrimmed_and_ignores_unknown_keys() {
    let config = parse(json!({
        "extra": true,
        "project": { "id": "demo", "note": 1, "rootMarkers": [{ "path": " services ", "x": null }] },
        "clients": [" alpha ", "not an identifier"],
        "targets": [{ "name": "api", "output": "  apps/api/.env  ", "comment": [] }],
        "tags": { "env": " env:{env} ", "other": null, "base": ["  b  "] },
    }))
    .unwrap();
    assert_eq!(config.targets[0].output, "  apps/api/.env  ");
    assert_eq!(
        config.clients,
        Some(strings(&[" alpha ", "not an identifier"]))
    );
    assert_eq!(
        config.project.unwrap().root_markers,
        Some(vec![RootMarker {
            path: Some(" services ".to_string()),
            package_json_name: None,
        }])
    );
    let tags = config.tags.unwrap();
    assert_eq!(tags.env.as_deref(), Some(" env:{env} "));
    assert_eq!(tags.base, Some(strings(&["  b  "])));
}

#[test]
fn duplicate_json_keys_keep_the_last_value() {
    let raw = r#"{"targets": 1, "targets": [{"name": "api", "output": "x"}], "project": {"id": "bad id", "id": "demo"}}"#;
    let config = parse_config(raw).unwrap();
    assert_eq!(config.targets, vec![target("api", "x")]);
    assert_eq!(config.project.unwrap().id.as_deref(), Some("demo"));
}

// --- 02 §8.2 row 2 ---------------------------------------------------------

#[test]
fn rejects_invalid_target_groups() {
    let raw = json!({ "targets": [{ "name": "api", "output": "apps/api/.env", "groups": ["all apis"] }] });
    let message = parse_error(&raw.to_string());
    assert!(message.contains("unsupported characters"));
    assert_eq!(
        message,
        "target api group all apis contains unsupported characters"
    );
}

// --- 02 §8.2 row 3 ---------------------------------------------------------

#[test]
fn rejects_malformed_config_fields_with_specific_messages() {
    assert!(parse_error("{").starts_with("config field is not valid JSON: "));
    assert_eq!(parse_error("[]"), "config JSON must be an object");
    assert_eq!(
        parse_error(&with_target(json!({ "clients": "alpha" })).to_string()),
        "config.clients must be an array"
    );
    assert_eq!(
        parse_error(&with_target(json!({ "project": { "id": "bad project" } })).to_string()),
        "project id contains unsupported characters"
    );
}

#[test]
fn invalid_json_is_a_user_error_with_the_oracle_prefix() {
    for raw in [
        "",
        "   ",
        "{",
        "[1,]",
        "{\"a\":1}x",
        "\u{feff}{}",
        "{'targets': []}",
        "NaN",
    ] {
        let message = parse_error(raw);
        let suffix = message
            .strip_prefix("config field is not valid JSON: ")
            .unwrap_or_else(|| panic!("{raw:?}: {message}"));
        assert!(!suffix.is_empty(), "{raw:?}");
    }
}

#[test]
fn root_must_be_an_object() {
    for raw in ["null", "1", "true", "\"x\"", "[]", "[{}]"] {
        assert_eq!(parse_error(raw), "config JSON must be an object", "{raw}");
    }
}

// --- 02 §8.2 row 4 ---------------------------------------------------------

#[test]
fn rejects_duplicate_targets() {
    let message = parse_error(
        &json!({
            "targets": [
                { "name": "api", "output": "apps/api/.env" },
                { "name": "api", "output": "apps/api-copy/.env" },
            ],
        })
        .to_string(),
    );
    assert!(message.contains("duplicate target"));
    assert_eq!(message, "config.targets contains duplicate target api");
}

// --- every message in 02 §1.3 ----------------------------------------------

#[test]
fn targets_messages() {
    let cases: &[(Value, &str)] = &[
        (json!({}), "config.targets must be a non-empty array"),
        (
            json!({ "targets": null }),
            "config.targets must be a non-empty array",
        ),
        (
            json!({ "targets": {} }),
            "config.targets must be a non-empty array",
        ),
        (
            json!({ "targets": [] }),
            "config.targets must be a non-empty array",
        ),
        (
            json!({ "targets": "api" }),
            "config.targets must be a non-empty array",
        ),
        (
            json!({ "targets": [1] }),
            "config.targets[0] must be an object",
        ),
        (
            json!({ "targets": [null] }),
            "config.targets[0] must be an object",
        ),
        (
            json!({ "targets": [[]] }),
            "config.targets[0] must be an object",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "x" }, "web"] }),
            "config.targets[1] must be an object",
        ),
        (
            json!({ "targets": [{ "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": null, "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": 5, "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "", "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": " \t\n", "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "\u{feff}", "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "\u{3000}", "output": "x" }] }),
            "config.targets[0].name must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "api" }] }),
            "config.targets[0].output must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": null }] }),
            "config.targets[0].output must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": ["x"] }] }),
            "config.targets[0].output must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "\u{a0}" }] }),
            "config.targets[0].output must be a non-empty string",
        ),
        // Name and output are both checked before the identifier rule.
        (
            json!({ "targets": [{ "name": "a b" }] }),
            "config.targets[0].output must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "a b", "output": "x" }] }),
            "target a b contains unsupported characters",
        ),
        (
            json!({ "targets": [{ "name": " api", "output": "x" }] }),
            "target  api contains unsupported characters",
        ),
        (
            json!({ "targets": [{ "name": "-api", "output": "x" }] }),
            "target -api contains unsupported characters",
        ),
        (
            json!({ "targets": [{ "name": "api\n", "output": "x" }] }),
            "target api\n contains unsupported characters",
        ),
        // U+0085 is not JS whitespace: non-empty, then fails the identifier rule.
        (
            json!({ "targets": [{ "name": "\u{85}", "output": "x" }] }),
            "target \u{85} contains unsupported characters",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "x", "groups": null }] }),
            "config.targets[0].groups must be an array",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "x", "groups": "apis" }] }),
            "config.targets[0].groups must be an array",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "x", "groups": [1] }] }),
            "config.targets[0].groups[0] must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "x", "groups": ["apis", " "] }] }),
            "config.targets[0].groups[1] must be a non-empty string",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": "x", "groups": ["apis", "bad group", "-x"] }] }),
            "target api group bad group contains unsupported characters",
        ),
        // Strings are all checked before any identifier.
        (
            json!({ "targets": [{ "name": "api", "output": "x", "groups": ["bad group", null] }] }),
            "config.targets[0].groups[1] must be a non-empty string",
        ),
    ];
    for (config, message) in cases {
        assert_eq!(parse_error(&config.to_string()), *message, "{config}");
    }
}

#[test]
fn project_messages() {
    let cases: &[(Value, &str)] = &[
        (
            json!({ "project": null }),
            "config.project must be an object",
        ),
        (json!({ "project": [] }), "config.project must be an object"),
        (
            json!({ "project": "demo" }),
            "config.project must be an object",
        ),
        (
            json!({ "project": { "id": null } }),
            "config.project.id must be a string",
        ),
        (
            json!({ "project": { "id": 5 } }),
            "config.project.id must be a string",
        ),
        (
            json!({ "project": { "id": ["demo"] } }),
            "config.project.id must be a string",
        ),
        (
            json!({ "project": { "id": "bad project" } }),
            "project id contains unsupported characters",
        ),
        (
            json!({ "project": { "id": " " } }),
            "project id contains unsupported characters",
        ),
        (
            json!({ "project": { "id": "_demo" } }),
            "project id contains unsupported characters",
        ),
        (
            json!({ "project": { "rootMarkers": null } }),
            "config.project.rootMarkers must be an array",
        ),
        (
            json!({ "project": { "rootMarkers": {} } }),
            "config.project.rootMarkers must be an array",
        ),
        (
            json!({ "project": { "rootMarkers": [1] } }),
            "config.project.rootMarkers[0] must be an object",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "path": "a" }, null] } }),
            "config.project.rootMarkers[1] must be an object",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "path": null }] } }),
            "config.project.rootMarkers[0].path must be a non-empty string",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "path": " " }] } }),
            "config.project.rootMarkers[0].path must be a non-empty string",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "path": 1, "packageJsonName": 2 }] } }),
            "config.project.rootMarkers[0].path must be a non-empty string",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "packageJsonName": null }] } }),
            "config.project.rootMarkers[0].packageJsonName must be a non-empty string",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "path": "a", "packageJsonName": "" }] } }),
            "config.project.rootMarkers[0].packageJsonName must be a non-empty string",
        ),
        (
            json!({ "project": { "rootMarkers": [{}] } }),
            "config.project.rootMarkers[0] must define path or packageJsonName",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "other": "services" }] } }),
            "config.project.rootMarkers[0] must define path or packageJsonName",
        ),
        // The id is validated before the markers.
        (
            json!({ "project": { "rootMarkers": 1, "id": "bad id" } }),
            "project id contains unsupported characters",
        ),
    ];
    for (extra, message) in cases {
        assert_eq!(
            parse_error(&with_target(extra.clone()).to_string()),
            *message,
            "{extra}"
        );
    }
}

#[test]
fn string_array_messages() {
    let cases: &[(Value, &str)] = &[
        (
            json!({ "clients": null }),
            "config.clients must be an array",
        ),
        (json!({ "clients": {} }), "config.clients must be an array"),
        (
            json!({ "clients": [""] }),
            "config.clients[0] must be a non-empty string",
        ),
        (
            json!({ "clients": ["alpha", 1] }),
            "config.clients[1] must be a non-empty string",
        ),
        (
            json!({ "environments": null }),
            "config.environments must be an array",
        ),
        (
            json!({ "environments": "local" }),
            "config.environments must be an array",
        ),
        (
            json!({ "environments": ["local", "\r\n"] }),
            "config.environments[1] must be a non-empty string",
        ),
    ];
    for (extra, message) in cases {
        assert_eq!(
            parse_error(&with_target(extra.clone()).to_string()),
            *message,
            "{extra}"
        );
    }
}

#[test]
fn tags_messages() {
    let cases: &[(Value, &str)] = &[
        (json!({ "tags": null }), "config.tags must be an object"),
        (json!({ "tags": [] }), "config.tags must be an object"),
        (json!({ "tags": "x" }), "config.tags must be an object"),
        (
            json!({ "tags": { "base": null } }),
            "config.tags.base must be an array",
        ),
        (
            json!({ "tags": { "base": "envswitch" } }),
            "config.tags.base must be an array",
        ),
        (
            json!({ "tags": { "base": ["a", ""] } }),
            "config.tags.base[1] must be a non-empty string",
        ),
        (
            json!({ "tags": { "project": null } }),
            "config.tags.project must be a non-empty string",
        ),
        (
            json!({ "tags": { "env": null } }),
            "config.tags.env must be a non-empty string",
        ),
        (
            json!({ "tags": { "client": 1 } }),
            "config.tags.client must be a non-empty string",
        ),
        (
            json!({ "tags": { "target": "" } }),
            "config.tags.target must be a non-empty string",
        ),
        (
            json!({ "tags": { "group": " " } }),
            "config.tags.group must be a non-empty string",
        ),
        (
            json!({ "tags": { "layer": [] } }),
            "config.tags.layer must be a non-empty string",
        ),
        // base first, then project, env, client, target, group, layer.
        (
            json!({ "tags": { "layer": "", "base": [1] } }),
            "config.tags.base[0] must be a non-empty string",
        ),
        (
            json!({ "tags": { "layer": "", "group": "", "target": "", "client": "", "env": "", "project": "" } }),
            "config.tags.project must be a non-empty string",
        ),
        (
            json!({ "tags": { "layer": "", "group": "", "env": "" } }),
            "config.tags.env must be a non-empty string",
        ),
        (
            json!({ "tags": { "layer": "", "group": "" } }),
            "config.tags.group must be a non-empty string",
        ),
    ];
    for (extra, message) in cases {
        assert_eq!(
            parse_error(&with_target(extra.clone()).to_string()),
            *message,
            "{extra}"
        );
    }
}

#[test]
fn null_is_never_treated_as_absent() {
    // 02 §1.2 probed table.
    let cases: &[(Value, &str)] = &[
        (
            json!({ "project": null }),
            "config.project must be an object",
        ),
        (
            json!({ "clients": null }),
            "config.clients must be an array",
        ),
        (json!({ "tags": null }), "config.tags must be an object"),
        (
            json!({ "tags": { "base": null } }),
            "config.tags.base must be an array",
        ),
        (
            json!({ "project": { "id": null } }),
            "config.project.id must be a string",
        ),
        (
            json!({ "project": { "rootMarkers": [{ "path": null }] } }),
            "config.project.rootMarkers[0].path must be a non-empty string",
        ),
        (
            json!({ "tags": { "env": null } }),
            "config.tags.env must be a non-empty string",
        ),
    ];
    for (extra, message) in cases {
        assert_eq!(
            parse_error(&with_target(extra.clone()).to_string()),
            *message,
            "{extra}"
        );
    }
    assert_eq!(
        parse_error(
            &json!({ "targets": [{ "name": "api", "output": "x", "groups": null }] }).to_string()
        ),
        "config.targets[0].groups must be an array"
    );
}

#[test]
fn checks_run_in_the_ts_order() {
    // Targets (in index order) before project, clients, environments, tags,
    // and duplicates, regardless of JSON key order.
    let raw = r#"{"targets":[{"name":"a","output":"x"},{"name":"a b","output":"y"}],"project":3}"#;
    assert_eq!(
        parse_error(raw),
        "target a b contains unsupported characters"
    );
    let raw = r#"{"tags":1,"environments":1,"clients":1,"project":1,"targets":[{"name":"a","output":"x"},{"name":"a","output":"y"}]}"#;
    assert_eq!(parse_error(raw), "config.project must be an object");
    let raw = r#"{"tags":1,"environments":1,"clients":1,"targets":[{"name":"a","output":"x"},{"name":"a","output":"y"}]}"#;
    assert_eq!(parse_error(raw), "config.clients must be an array");
    let raw = r#"{"tags":1,"environments":1,"targets":[{"name":"a","output":"x"},{"name":"a","output":"y"}]}"#;
    assert_eq!(parse_error(raw), "config.environments must be an array");
    let raw = r#"{"tags":1,"targets":[{"name":"a","output":"x"},{"name":"a","output":"y"}]}"#;
    assert_eq!(parse_error(raw), "config.tags must be an object");
    let raw = r#"{"targets":[{"name":"a","output":"x"},{"name":"a","output":"y"},{"name":"a","output":"z"}]}"#;
    assert_eq!(
        parse_error(raw),
        "config.targets contains duplicate target a"
    );
    // The root object check comes before targets; targets before anything else.
    assert_eq!(
        parse_error(r#"{"project":1}"#),
        "config.targets must be a non-empty array"
    );
    let raw = r#"{"targets":[{"name":"a b","output":"x"},1]}"#;
    assert_eq!(
        parse_error(raw),
        "target a b contains unsupported characters"
    );
}

#[test]
fn parse_errors_exit_with_code_1() {
    for raw in ["{", "[]", r#"{"targets":[]}"#] {
        assert_eq!(parse_config(raw).unwrap_err().exit_code(), 1, "{raw}");
    }
}

// --- selection (02 §1.5) and 02 §8.2 row 8 ---------------------------------

fn two_targets() -> Config {
    parse(json!({
        "targets": [
            { "name": "api", "output": "apps/api/.env" },
            { "name": "web", "output": "apps/web/.env" },
        ],
    }))
    .unwrap()
}

#[test]
fn selects_all_targets_or_one_named_target() {
    let config = two_targets();
    assert_eq!(select_targets(&config, "all").unwrap().len(), 2);
    let names: Vec<String> = select_targets(&config, "web")
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["web"]);
    let message = select_targets(&config, "worker").unwrap_err().to_string();
    assert!(message.contains("not defined"));
    assert_eq!(message, "target worker is not defined in config.targets");
}

#[test]
fn select_all_keeps_config_order_and_skips_the_identifier_check() {
    let config = two_targets();
    assert_eq!(select_targets(&config, "all").unwrap(), config.targets);
    for name in ["a b", "", "-x", "api\n", "All"] {
        let expected = if name == "All" {
            "target All is not defined in config.targets".to_string()
        } else {
            "target contains unsupported characters".to_string()
        };
        assert_eq!(
            select_targets(&config, name),
            Err(Error::User(expected)),
            "{name:?}"
        );
    }
}

#[test]
fn a_target_named_all_cannot_be_selected_alone() {
    let config = parse(
        json!({ "targets": [{ "name": "all", "output": "a" }, { "name": "api", "output": "b" }] }),
    )
    .unwrap();
    assert_eq!(select_targets(&config, "all").unwrap().len(), 2);
}

#[test]
fn validate_selection_checks_identifiers_then_membership() {
    let open = two_targets();
    assert_eq!(validate_selection(&open, "local", "alpha"), Ok(()));
    assert_eq!(
        validate_selection(&open, "a b", "c d"),
        Err(Error::user("environment contains unsupported characters"))
    );
    assert_eq!(
        validate_selection(&open, "local", "c d"),
        Err(Error::user("client contains unsupported characters"))
    );
    assert_eq!(
        validate_selection(&open, "", "alpha"),
        Err(Error::user("environment contains unsupported characters"))
    );

    let listed = parse(with_target(
        json!({ "environments": ["local"], "clients": ["alpha", "beta"] }),
    ))
    .unwrap();
    assert_eq!(validate_selection(&listed, "local", "beta"), Ok(()));
    assert_eq!(
        validate_selection(&listed, "dev", "gamma"),
        Err(Error::user(
            "environment dev is not listed in config.environments"
        ))
    );
    assert_eq!(
        validate_selection(&listed, "local", "gamma"),
        Err(Error::user("client gamma is not listed in config.clients"))
    );
    // Both identifier checks run before either membership check.
    assert_eq!(
        validate_selection(&listed, "dev", "c d"),
        Err(Error::user("client contains unsupported characters"))
    );
    // Membership is exact (no trimming, case-sensitive).
    assert_eq!(
        validate_selection(&listed, "Local", "alpha"),
        Err(Error::user(
            "environment Local is not listed in config.environments"
        ))
    );

    // Empty lists allow anything.
    let empty = parse(with_target(json!({ "environments": [], "clients": [] }))).unwrap();
    assert_eq!(validate_selection(&empty, "dev", "gamma"), Ok(()));
}

#[test]
fn non_identifier_list_entries_parse_but_can_never_be_selected() {
    let config = parse(with_target(json!({ "clients": ["a b"] }))).unwrap();
    assert_eq!(
        validate_selection(&config, "local", "a b"),
        Err(Error::user("client contains unsupported characters"))
    );
}
