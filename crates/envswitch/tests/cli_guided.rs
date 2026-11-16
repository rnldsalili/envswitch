mod support;

use std::sync::Arc;
use std::time::Duration;

use envswitch::error::Error;
use support::{
    Answer, ConcurrencyGate, FakeOp, FakeOpFactory, FakeVaultLister, OpCall, PromptRecord,
    ScriptedPrompter, TempProject, env_map, item, vault,
};

const CONFIG: &str = r#"{"project":{"id":"demo"},"clients":["alpha","beta"],"environments":["local","dev"],"targets":[{"name":"api","output":"services/api/.env","groups":["apis"]},{"name":"web","output":"apps/web/.env","groups":["apps"]}],"tags":{"base":["envswitch"]}}"#;

fn config_op(id: &str) -> FakeOp {
    FakeOp::new(id)
        .with_list_for(
            &["envswitch-config"],
            Ok(vec![item("cfg", "demo config", &["envswitch-config"])]),
        )
        .with_config("cfg", CONFIG)
        .with_items(vec![item(
            "client-item",
            "beta-client",
            &[
                "envswitch",
                "project:demo",
                "env:dev",
                "layer:client",
                "client:beta",
            ],
        )])
        .with_values("client-item", env_map(&[("CLIENT_ONLY", "1")]))
}

#[test]
fn rejects_guided_mode_without_an_interactive_terminal_before_any_op_call() {
    let project = TempProject::new();
    for (stdin, stdout) in [(false, false), (true, false), (false, true)] {
        let mut test = support::deps(project.root()).with_vaults(FakeVaultLister::forbidden());
        test.deps.stdin_is_tty = stdin;
        test.deps.stdout_is_tty = stdout;
        assert_eq!(test.run(&[]), 1);
        assert_eq!(
            test.err.text(),
            "envswitch: plain envswitch requires an interactive terminal; use envswitch switch --vault <vault> --config-item <item> --client <client> for scripts\n"
        );
        assert!(test.out.bytes().is_empty());
    }
}

#[test]
fn rejects_no_config_vaults_and_skips_all_errors_in_vault_order() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([]);
    let factory = FakeOpFactory::new()
        .with(FakeOp::new("empty"))
        .with(FakeOp::new("bad").with_list_error(Error::internal("inaccessible")))
        .with(FakeOp::new("bad-user").with_list_error(Error::user("bad data")));
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(factory)
        .with_vaults(FakeVaultLister::new(vec![
            vault("empty", "Empty"),
            vault("bad", "Bad Vault"),
            vault("bad-user", "Bad Data"),
        ]));
    assert_eq!(test.run(&[]), 1);
    assert_eq!(
        test.err.text(),
        "envswitch: no 1Password vaults contain items tagged envswitch-config\n"
    );
    assert_eq!(
        test.events.lines(),
        [
            "info:listing 1Password vaults",
            "done:found 3 vault(s)",
            "info:checking vaults for config items tagged envswitch-config",
            "info:skipping vault Bad Vault (could not list items)",
            "info:skipping vault Bad Data (could not list items)",
            "done:found 0 vault(s) with config items",
        ]
    );
    assert!(prompt.records().is_empty());
}

#[test]
fn scans_in_parallel_preserves_factory_order_and_reuses_selected_client() {
    let project = TempProject::new();
    let gate = ConcurrencyGate::new(2, Duration::from_secs(3));
    let first = FakeOp::new("vault-a").with_gate(Arc::clone(&gate));
    let second = FakeOp::new("vault-b")
        .with_gate(Arc::clone(&gate))
        .with_items(vec![item("cfg", "Chosen", &["envswitch-config"])])
        .with_config_error("cfg", Error::Cancelled);
    let prompt = ScriptedPrompter::new([]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(FakeOpFactory::new().with(first).with(second.clone()))
        .with_vaults(FakeVaultLister::new(vec![
            vault("vault-a", "A"),
            vault("vault-b", "B"),
        ]));
    assert_eq!(test.run(&[]), 130);
    assert_eq!(gate.arrived(), 2);
    assert_eq!(test.factory.created(), ["vault-a", "vault-b"]);
    assert_eq!(
        second.calls(),
        [
            OpCall::ListItemsByTags(vec!["envswitch-config".into()]),
            OpCall::ReadConfigItem("cfg".into())
        ]
    );
    assert!(test.err.bytes().is_empty());
    assert!(prompt.records().is_empty());
    assert_eq!(
        test.events.lines().last().unwrap(),
        "info:reading config item Chosen"
    );
}

#[test]
fn prompt_cancellation_returns_130_without_an_error_line() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::cancelling();
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(
            FakeOpFactory::new()
                .with(config_op("a"))
                .with(config_op("b")),
        )
        .with_vaults(FakeVaultLister::new(vec![vault("a", "A"), vault("b", "B")]));
    assert_eq!(test.run(&[]), 130);
    assert_eq!(prompt.messages(), ["Vault"]);
    assert!(test.err.bytes().is_empty());
}

#[test]
fn vault_and_config_labels_preserve_empty_and_duplicate_id_fallbacks() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([Answer::Index(1), Answer::Cancel]);
    let op = FakeOp::new("by-name").with_items(vec![
        item("id", "", &[]),
        item("same", "same", &[]),
        item("", "title", &[]),
    ]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(FakeOpFactory::new().with(config_op("first")).with(op))
        .with_vaults(FakeVaultLister::new(vec![
            vault("first", "first"),
            vault("", "by-name"),
        ]));
    assert_eq!(test.run(&[]), 130);
    assert_eq!(test.factory.created(), ["first", "by-name"]);
    assert_eq!(
        prompt.records(),
        [
            PromptRecord::Select {
                message: "Vault".into(),
                labels: vec!["first".into(), "by-name".into()],
                default_index: None
            },
            PromptRecord::Select {
                message: "Config item".into(),
                labels: vec!["id (id)".into(), "same".into(), "title".into()],
                default_index: None
            },
        ]
    );
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn runs_guided_switch_with_full_prompt_sequence_and_title_logs() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([
        Answer::label("beta"),
        Answer::label("dev"),
        Answer::label("api"),
        Answer::label("Dry run"),
    ]);
    let op = config_op("vault-a");
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(
            FakeOpFactory::new()
                .with(op.clone())
                .with(FakeOp::new("vault-b")),
        )
        .with_vaults(FakeVaultLister::new(vec![
            vault("vault-a", "Vault A"),
            vault("vault-b", "Vault B"),
        ]));
    assert_eq!(test.run(&[]), 0, "{}", test.err.text());
    assert_eq!(test.factory.created(), ["vault-a", "vault-b"]);
    assert_eq!(
        prompt.messages(),
        ["Client", "Environment", "Target", "Mode"]
    );
    assert_eq!(
        prompt.records(),
        [
            PromptRecord::Select {
                message: "Client".into(),
                labels: vec!["alpha".into(), "beta".into()],
                default_index: None
            },
            PromptRecord::Select {
                message: "Environment".into(),
                labels: vec!["local".into(), "dev".into()],
                default_index: Some(0)
            },
            PromptRecord::Select {
                message: "Target".into(),
                labels: vec!["all".into(), "api".into(), "web".into()],
                default_index: Some(0)
            },
            PromptRecord::Select {
                message: "Mode".into(),
                labels: vec!["Dry run".into(), "Write .env files".into()],
                default_index: Some(1)
            },
        ]
    );
    assert!(op.tag_queries().contains(&vec![
        "envswitch".into(),
        "project:demo".into(),
        "env:dev".into()
    ]));
    assert!(op.calls().contains(&OpCall::ReadConfigItem("cfg".into())));
    assert!(
        test.events
            .lines()
            .contains(&"info:reading config item demo config".into())
    );
    assert!(test.out.lines().contains(&"envswitch guided switch".into()));
    assert!(
        test.out
            .lines()
            .contains(&"Dry run 1 target(s) from envswitch-project".into())
    );
    assert!(
        test.out
            .lines()
            .contains(&"  layers: client:beta-client".into())
    );
    assert!(project.read("services/api/.env").is_none());
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn skips_inaccessible_vaults_and_still_completes_a_guided_write() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([
        Answer::label("alpha"),
        Answer::label("local"),
        Answer::label("all"),
    ]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(
            FakeOpFactory::new()
                .with(FakeOp::new("bad").with_list_error(Error::internal("no access")))
                .with(config_op("good")),
        )
        .with_vaults(FakeVaultLister::new(vec![
            vault("bad", "Bad Vault"),
            vault("good", "Good"),
        ]));
    assert_eq!(test.run(&[]), 0, "{}", test.err.text());
    assert!(
        test.events
            .lines()
            .contains(&"info:skipping vault Bad Vault (could not list items)".into())
    );
    assert!(project.read("services/api/.env").is_some());
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn write_mode_is_the_guided_default() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([
        Answer::label("alpha"),
        Answer::label("local"),
        Answer::label("all"),
    ]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(FakeOpFactory::new().with(config_op("a")))
        .with_vaults(FakeVaultLister::new(vec![vault("a", "A")]));
    assert_eq!(test.run(&[]), 0, "{}", test.err.text());
    assert_eq!(
        prompt.records().last(),
        Some(&PromptRecord::Select {
            message: "Mode".into(),
            labels: vec!["Dry run".into(), "Write .env files".into()],
            default_index: Some(1)
        })
    );
    assert!(
        test.out
            .lines()
            .contains(&"Updated 2 target(s) from envswitch-project".into())
    );
    assert!(project.read("services/api/.env").is_some());
    assert!(project.read("apps/web/.env").is_some());
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn input_fallback_has_required_and_environment_default_and_single_target_is_automatic() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([
        Answer::text("alpha"),
        Answer::Default,
        Answer::label("Dry run"),
    ]);
    let op = config_op("a").with_config(
        "cfg",
        r#"{"targets":[{"name":"api","output":"services/api/.env"}]}"#,
    );
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(FakeOpFactory::new().with(op))
        .with_vaults(FakeVaultLister::new(vec![vault("a", "A")]));
    assert_eq!(test.run(&[]), 0, "{}", test.err.text());
    assert_eq!(
        prompt.records(),
        [
            PromptRecord::Input {
                message: "Client".into(),
                default: None,
                required: true
            },
            PromptRecord::Input {
                message: "Environment".into(),
                default: Some("local".into()),
                required: true
            },
            PromptRecord::Select {
                message: "Mode".into(),
                labels: vec!["Dry run".into(), "Write .env files".into()],
                default_index: Some(1)
            },
        ]
    );
    assert!(
        test.out
            .lines()
            .contains(&"Dry run 1 target(s) from envswitch-project".into())
    );
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn multiple_choices_show_vault_and_selected_vaults_config_items() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([
        Answer::label("Vault B (vault-b)"),
        Answer::label("vault-b config (cfg-b)"),
        Answer::label("alpha"),
        Answer::label("local"),
        Answer::label("all"),
    ]);
    let mut factory = FakeOpFactory::new();
    for id in ["vault-a", "vault-b"] {
        factory = factory.with(
            FakeOp::new(id)
                .with_items(vec![
                    item(&format!("cfg-{}", &id[6..]), &format!("{id} config"), &[]),
                    item(
                        &format!("cfg-{}-other", &id[6..]),
                        &format!("{id} other config"),
                        &[],
                    ),
                ])
                .with_config(&format!("cfg-{}", &id[6..]), CONFIG),
        );
    }
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_factory(factory)
        .with_vaults(FakeVaultLister::new(vec![
            vault("vault-a", "Vault A"),
            vault("vault-b", "Vault B"),
        ]));
    assert_eq!(test.run(&[]), 0, "{}", test.err.text());
    assert_eq!(
        prompt.messages(),
        [
            "Vault",
            "Config item",
            "Client",
            "Environment",
            "Target",
            "Mode"
        ]
    );
    assert_eq!(
        prompt.records()[1],
        PromptRecord::Select {
            message: "Config item".into(),
            labels: vec![
                "vault-b config (cfg-b)".into(),
                "vault-b other config (cfg-b-other)".into()
            ],
            default_index: None
        }
    );
    assert!(
        test.out
            .lines()
            .contains(&"Updated 2 target(s) from envswitch-project".into())
    );
}
