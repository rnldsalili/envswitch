use std::sync::{Arc, Mutex};

use envswitch::error::{Error, Result};
use envswitch::model::{ItemSummary, Vault};
use envswitch::op::{OnePasswordClient, OpClientFactory, OpVaultLister, list_vaults};
use envswitch::ports::{CommandOutput, CommandRunner, OnePassword, OpFactory, VaultLister};

#[derive(Default)]
struct StaticRunner {
    response: CommandOutput,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl StaticRunner {
    fn json(raw: &str) -> Arc<Self> {
        Arc::new(Self {
            response: CommandOutput {
                code: Some(0),
                stdout: raw.as_bytes().to_vec(),
                stderr: Vec::new(),
            },
            ..Self::default()
        })
    }

    fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl CommandRunner for StaticRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        self.calls
            .lock()
            .unwrap()
            .push((program.into(), args.into()));
        Ok(self.response.clone())
    }
}

fn call(args: &[&str]) -> (String, Vec<String>) {
    ("op".into(), args.iter().map(|arg| (*arg).into()).collect())
}

fn item(id: &str, title: &str) -> ItemSummary {
    ItemSummary {
        id: id.into(),
        title: title.into(),
        tags: Vec::new(),
    }
}

#[test]
fn authentication_ignores_stdout_and_uses_exact_argv() {
    let runner = StaticRunner::json("not JSON");
    let factory = OpClientFactory::new(runner.clone());
    assert!(runner.calls().is_empty());
    let op = factory.create("vault");
    op.assert_authenticated().unwrap();
    assert_eq!(runner.calls(), [call(&["whoami", "--format", "json"])]);
}

#[test]
#[ignore = "needs C1"]
fn vaults_are_lenient_and_use_exact_argv() {
    let runner = StaticRunner::json(
        r#"[{"id":"vault-a","name":"Vault A"},{"id":"fallback","name":3},{"name":"name only"}]"#,
    );
    let vaults = OpVaultLister::new(runner.clone()).list_vaults().unwrap();
    assert_eq!(
        vaults,
        [
            Vault {
                id: "vault-a".into(),
                name: "Vault A".into()
            },
            Vault {
                id: "fallback".into(),
                name: "fallback".into()
            },
            Vault {
                id: "".into(),
                name: "name only".into()
            }
        ]
    );
    assert_eq!(
        runner.calls(),
        [call(&["vault", "list", "--format", "json"])]
    );
}

#[test]
#[ignore = "needs C1"]
fn lists_items_in_api_order_and_filters_tags_without_normalizing_strings() {
    let runner = StaticRunner::json(
        r#"[{"id":"i","title":false,"tags":["a",1,null,"","b"]},{"id":2,"title":"  title  ","tags":"ignored"}]"#,
    );
    let op = OnePasswordClient::new("Vault A", runner.clone());
    let items = op
        .list_items_by_tags(&["one,two".into(), "three".into()])
        .unwrap();
    assert_eq!(
        items,
        [
            ItemSummary {
                id: "i".into(),
                title: "".into(),
                tags: vec!["a".into(), "".into(), "b".into()]
            },
            item("", "  title  ")
        ]
    );
    op.list_items_by_tags(&[]).unwrap();
    assert_eq!(
        runner.calls(),
        [
            call(&[
                "item",
                "list",
                "--vault",
                "Vault A",
                "--format",
                "json",
                "--tags",
                "one,two,three"
            ]),
            call(&["item", "list", "--vault", "Vault A", "--format", "json"])
        ]
    );
}

#[test]
#[ignore = "needs C1"]
fn rejects_malformed_lists_with_user_errors() {
    for (raw, expected) in [
        ("{}", "op item list returned non-array JSON"),
        ("null", "op item list returned non-array JSON"),
        ("[null]", "op item list result[0] must be an object"),
        ("[[]]", "op item list result[0] must be an object"),
        (
            r#"[{"id":4,"title":false}]"#,
            "op item list result[0] must include id or title",
        ),
    ] {
        let op = OnePasswordClient::new("vault", StaticRunner::json(raw));
        assert_eq!(
            op.list_items_by_tags(&[]).unwrap_err(),
            Error::user(expected)
        );
    }
    for (raw, expected) in [
        ("{}", "op vault list returned non-array JSON"),
        ("[null]", "op vault list result[0] must be an object"),
        ("[[]]", "op vault list result[0] must be an object"),
        ("[{}]", "op vault list result[0] must include id or name"),
    ] {
        assert_eq!(
            list_vaults(StaticRunner::json(raw).as_ref()).unwrap_err(),
            Error::user(expected)
        );
    }
    let op = OnePasswordClient::new("vault", StaticRunner::json("not-json"));
    let error = op.list_items_by_tags(&[]).unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert!(
        error
            .to_string()
            .starts_with("op item list output is not valid JSON: ")
    );
    let error = list_vaults(StaticRunner::json("").as_ref()).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("op vault list output is not valid JSON: ")
    );
}

#[test]
#[ignore = "needs C1"]
fn config_requires_first_exact_label_with_nonempty_value_and_returns_it_untrimmed() {
    for raw in [
        r#"{"fields":[]}"#,
        r#"{"fields":[{"label":" config","value":"{}"}]}"#,
        r#"{"fields":[{"label":"config","value":""}]}"#,
        r#"{"fields":[{"label":"config","value":" \t\ufeff"}]}"#,
        r#"{"fields":[{"label":"config","value":3}]}"#,
        r#"{"fields":[{"label":"config"},{"label":"config","value":"{}"}]}"#,
    ] {
        let op = OnePasswordClient::new("vault", StaticRunner::json(raw));
        assert_eq!(
            op.read_config_item("cfg").unwrap_err(),
            Error::user("1Password config item cfg must contain a non-empty field named config")
        );
    }
    let runner = StaticRunner::json(
        r#"{"fields":[{"label":" config","value":"wrong"},{"label":"config","value":"  {}  "},{"label":"config","value":"wrong"}]}"#,
    );
    let op = OnePasswordClient::new("vault", runner.clone());
    assert_eq!(op.read_config_item("-My Title").unwrap(), "  {}  ");
    assert_eq!(
        runner.calls(),
        [call(&[
            "item",
            "get",
            "-My Title",
            "--vault",
            "vault",
            "--format",
            "json",
            "--reveal"
        ])]
    );
}

#[test]
#[ignore = "needs C1"]
fn env_fields_use_labels_trim_and_last_duplicate_wins_at_first_position() {
    let runner = StaticRunner::json(
        r#"{"fields":[
        {"label":"\ufeff VALID_KEY \u00a0","value":" 1 ","id":"not_the_key","type":"CONCEALED","section":{"id":"s1"}},
        {"label":"invalid-key","value":"2"},{"label":" notesPlain ","value":"ignored"},
        {"label":"9KEY"},{"label":""},{"value":"no label"},{"label":3},
        {"label":"NOVAL"},{"label":"NONSTRING","value":5},{"label":"SPACED","value":"  kept  "},
        {"label":"username","value":"user"},{"label":"password","value":"secret"},
        {"label":"OTP","value":"otpauth://uri","totp":"123456","type":"OTP"},
        {"label":"__proto__","value":"kept"},
        {"label":"VALID_KEY","value":"last","section":{"id":"s2"}}
    ]}"#,
    );
    let op = OnePasswordClient::new("vault", runner.clone());
    let values = op.read_env_item(&item("env-id", "unused title")).unwrap();
    let pairs: Vec<_> = values
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("VALID_KEY", "last"),
            ("NOVAL", ""),
            ("NONSTRING", ""),
            ("SPACED", "  kept  "),
            ("username", "user"),
            ("password", "secret"),
            ("OTP", "otpauth://uri"),
            ("__proto__", "kept")
        ]
    );
    op.read_env_item(&item("", "My Title")).unwrap();
    assert_eq!(
        runner.calls(),
        [
            call(&[
                "item", "get", "env-id", "--vault", "vault", "--format", "json", "--reveal"
            ]),
            call(&[
                "item", "get", "My Title", "--vault", "vault", "--format", "json", "--reveal"
            ])
        ]
    );
}

#[test]
#[ignore = "needs C1"]
fn rejects_bad_item_details_and_validates_all_fields_before_selecting_config() {
    for raw in ["null", "[]", "{}", r#"{"fields":null}"#] {
        let op = OnePasswordClient::new("vault", StaticRunner::json(raw));
        assert_eq!(
            op.read_config_item("cfg").unwrap_err(),
            Error::user("op item get cfg returned unexpected JSON")
        );
    }
    for raw in [r#"{"fields":[null]}"#, r#"{"fields":[[]]}"#] {
        let op = OnePasswordClient::new("vault", StaticRunner::json(raw));
        assert_eq!(
            op.read_env_item(&item("env", "")).unwrap_err(),
            Error::user("op item field[0] must be an object")
        );
    }
    let op = OnePasswordClient::new(
        "vault",
        StaticRunner::json(r#"{"fields":[{"label":"config","value":"{}"},false]}"#),
    );
    assert_eq!(
        op.read_config_item("cfg").unwrap_err(),
        Error::user("op item field[1] must be an object")
    );
    let op = OnePasswordClient::new("vault", StaticRunner::json("not-json"));
    assert!(
        op.read_config_item("cfg")
            .unwrap_err()
            .to_string()
            .starts_with("op item get cfg output is not valid JSON: ")
    );
}

#[test]
#[ignore = "needs C1"]
fn failed_commands_use_trimmed_stderr_or_safe_command_fallback() {
    for stderr in [" \u{feff}safe failure\u{00a0}\n", "", " \t\n"] {
        let runner = Arc::new(StaticRunner {
            response: CommandOutput {
                code: Some(1),
                stdout: b"SUPER_SECRET".to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            },
            ..StaticRunner::default()
        });
        let op = OnePasswordClient::new("vault", runner.clone());
        for (error, command) in [
            (
                op.assert_authenticated().unwrap_err(),
                "Command failed: op whoami --format json",
            ),
            (
                op.list_items_by_tags(&[]).unwrap_err(),
                "Command failed: op item list --vault vault --format json",
            ),
            (
                op.read_config_item("cfg").unwrap_err(),
                "Command failed: op item get cfg --vault vault --format json --reveal",
            ),
            (
                op.read_env_item(&item("env", "")).unwrap_err(),
                "Command failed: op item get env --vault vault --format json --reveal",
            ),
            (
                list_vaults(runner.as_ref()).unwrap_err(),
                "Command failed: op vault list --format json",
            ),
        ] {
            assert_eq!(error.exit_code(), 2);
            let message = error.to_string();
            assert!(!message.contains("SUPER_SECRET"));
            if stderr.contains("safe failure") {
                assert_eq!(message, "safe failure");
            } else {
                assert_eq!(message, command);
            }
        }
    }
}
