mod support;

use envswitch::error::Error;
use support::{FakeOp, FakeOpFactory, OpCall, TempProject, env_map, item};

const CONFIG: &str = r#"{"clients":["alpha","beta"],"targets":[{"name":"api","output":"services/api/.env"},{"name":"web","output":"apps/web/.env"}]}"#;

fn command_deps(project: &TempProject, config: &str) -> support::TestDeps {
    support::deps(project.root())
        .with_factory(FakeOpFactory::new().with(FakeOp::new("v").with_config("cfg", config)))
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn string_flags_preserve_untrimmed_values_and_report_errors_in_statement_order() {
    let project = TempProject::new();
    for (argv, error) in [
        (vec!["switch", "--vault="], "--vault requires a value"),
        (
            vec!["switch", "--vault=\u{feff}\u{a0}"],
            "--vault requires a value",
        ),
        (vec!["switch", "--vault", "v"], "--config-item is required"),
        (
            vec!["switch", "--vault", "v", "--config-item", "cfg"],
            "--client is required",
        ),
        (
            vec![
                "switch",
                "--vault",
                "v",
                "--config-item",
                "cfg",
                "--client",
                "alpha",
                "--env",
            ],
            "--env requires a value",
        ),
        (
            vec![
                "switch",
                "--vault",
                "v",
                "--config-item",
                "cfg",
                "--client",
                "alpha",
                "--target=",
            ],
            "--target requires a value",
        ),
    ] {
        let mut test = command_deps(&project, CONFIG);
        assert_eq!(test.run(&argv), 1, "{argv:?}");
        assert_eq!(test.err.text(), format!("envswitch: {error}\n"));
        assert!(test.factory.created().is_empty());
    }
    let op = FakeOp::new(" v ").with_config_error(" cfg ", Error::internal("read reached"));
    let mut test =
        support::deps(project.root()).with_factory(FakeOpFactory::new().with(op.clone()));
    assert_eq!(
        test.run(&["list", "--vault", " v ", "--config-item", " cfg "]),
        2
    );
    assert_eq!(test.factory.created(), [" v "]);
    assert_eq!(op.calls(), [OpCall::ReadConfigItem(" cfg ".into())]);
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn list_subjects_keep_raw_target_paths_and_empty_clients() {
    let project = TempProject::new();
    for (subject, expected) in [
        ("clients", "alpha\nbeta\n"),
        ("targets", "api\tservices/api/.env\nweb\tapps/web/.env\n"),
        (
            "layers",
            "shared\tlayer:shared\ngroup\tlayer:group\ntarget\tlayer:target\nclient\tlayer:client\nclient-target\tlayer:client-target\n",
        ),
    ] {
        let mut test = command_deps(&project, CONFIG);
        assert_eq!(
            test.run(&[
                "list",
                subject,
                "--vault",
                "v",
                "--config-item",
                "cfg",
                "ignored-extra"
            ]),
            0,
            "{}",
            test.err.text()
        );
        assert_eq!(test.out.text(), expected);
        assert!(test.err.bytes().is_empty());
        assert_eq!(
            test.events.lines(),
            [
                "info:reading config item cfg",
                "done:loaded config item cfg"
            ]
        );
    }
    for clients in ["", ",\"clients\":[]"] {
        let config = format!("{{\"targets\":[{{\"name\":\"api\",\"output\":\".env\"}}]{clients}}}");
        let mut test = command_deps(&project, &config);
        assert_eq!(
            test.run(&["list", "clients", "--vault", "v", "--config-item", "cfg"]),
            0
        );
        assert_eq!(test.out.text(), "(none)\n");
    }
    let mut test = command_deps(&project, CONFIG);
    assert_eq!(
        test.run(&["list", "--vault", "v", "--config-item", "cfg"]),
        0
    );
    assert_eq!(
        test.out.text(),
        "api\tservices/api/.env\nweb\tapps/web/.env\n"
    );
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn list_loads_config_before_validating_its_subject() {
    let project = TempProject::new();
    let mut test = command_deps(&project, CONFIG);
    assert_eq!(
        test.run(&["list", "bogus", "--vault", "v", "--config-item", "cfg"]),
        1
    );
    assert_eq!(test.err.text(), "envswitch: unknown list subject: bogus\n");
    assert_eq!(
        test.factory.op("v").calls(),
        [OpCall::ReadConfigItem("cfg".into())]
    );
    assert_eq!(
        test.events.lines(),
        [
            "info:reading config item cfg",
            "done:loaded config item cfg"
        ]
    );
    let mut test = command_deps(&project, "invalid json");
    assert_eq!(
        test.run(&["list", "bogus", "--vault", "v", "--config-item", "cfg"]),
        1
    );
    assert!(!test.err.text().contains("unknown list subject"));
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn doctor_reports_authentication_and_parent_status_in_config_order() {
    let project = TempProject::new();
    project.dir("services/api");
    let mut test = command_deps(&project, CONFIG);
    assert_eq!(
        test.run(&["doctor", "--vault", "v", "--config-item", "cfg", "ignored"]),
        0,
        "{}",
        test.err.text()
    );
    let normalized = test
        .out
        .text()
        .replace(&project.root().display().to_string(), "{ROOT}")
        .replace('\\', "/");
    insta::assert_snapshot!(normalized, @r"
    1Password authentication: ok
    project root: {ROOT}
    targets: 2
    - api -> {ROOT}/services/api/.env (ok)
    - web -> {ROOT}/apps/web/.env (missing parent)
    ");
    assert_eq!(
        test.factory.op("v").calls(),
        [
            OpCall::AssertAuthenticated,
            OpCall::ReadConfigItem("cfg".into())
        ]
    );
    assert_eq!(
        test.events.lines(),
        [
            "info:checking 1Password authentication".to_string(),
            "done:1Password authentication ok".into(),
            "info:reading config item cfg".into(),
            "done:loaded config item cfg".into(),
            format!("info:inspecting project root {}", project.root().display()),
        ]
    );
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn doctor_auth_failure_stops_before_loading_config_and_returns_two() {
    let project = TempProject::new();
    let op = FakeOp::new("v").with_auth_error(Error::internal("sign in first"));
    let mut test =
        support::deps(project.root()).with_factory(FakeOpFactory::new().with(op.clone()));
    assert_eq!(
        test.run(&["doctor", "--vault", "v", "--config-item", "cfg"]),
        2
    );
    assert_eq!(test.err.text(), "envswitch: sign in first\n");
    assert_eq!(op.calls(), [OpCall::AssertAuthenticated]);
    assert!(test.out.bytes().is_empty());
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn switch_defaults_write_all_targets_and_dry_run_summaries_include_overwrites() {
    let project = TempProject::new();
    let op = FakeOp::new("v")
        .with_config("cfg", CONFIG)
        .with_items(vec![
            item("base", "base", &["env:local", "layer:shared"]),
            item(
                "client",
                "alpha",
                &["env:local", "client:alpha", "layer:client"],
            ),
        ])
        .with_values("base", env_map(&[("A", "old")]))
        .with_values("client", env_map(&[("A", "new"), ("B", "two")]));
    let mut test =
        support::deps(project.root()).with_factory(FakeOpFactory::new().with(op.clone()));
    assert_eq!(
        test.run(&[
            "switch",
            "--vault",
            "v",
            "--config-item",
            "cfg",
            "--client",
            "alpha",
            "--dry-run"
        ]),
        0,
        "{}",
        test.err.text()
    );
    let normalized = test
        .out
        .text()
        .replace(&project.root().display().to_string(), "{ROOT}")
        .replace('\\', "/");
    insta::assert_snapshot!(normalized, @r"
    Dry run 2 target(s) from envswitch-project
    - api -> {ROOT}/services/api/.env
      keys: 2
      layers: shared:base, client:alpha
      overwritten: A
    - web -> {ROOT}/apps/web/.env
      keys: 2
      layers: shared:base, client:alpha
      overwritten: A
    ");
    assert!(project.read("services/api/.env").is_none());
    assert_eq!(op.tag_queries(), [vec!["env:local"]]);
    assert_eq!(op.read_counts().get("base"), Some(&1));
    assert_eq!(op.read_counts().get("client"), Some(&1));
    test.out.clear();
    assert_eq!(
        test.run(&[
            "switch",
            "--vault",
            "v",
            "--config-item",
            "cfg",
            "--client",
            "alpha"
        ]),
        0
    );
    assert!(
        test.out
            .text()
            .starts_with("Updated 2 target(s) from envswitch-project\n")
    );
    assert_eq!(
        project.read("services/api/.env").as_deref(),
        Some("A=new\nB=two\n")
    );
    assert_eq!(
        project.read("apps/web/.env").as_deref(),
        Some("A=new\nB=two\n")
    );
}

#[test]
#[ignore = "needs C1/C2/C3"]
fn empty_layers_summary_says_none_and_omits_overwrites() {
    let project = TempProject::new();
    let mut test = command_deps(&project, CONFIG);
    assert_eq!(
        test.run(&[
            "switch",
            "--vault",
            "v",
            "--config-item",
            "cfg",
            "--client",
            "alpha",
            "--target",
            "api",
            "--dry-run"
        ]),
        0
    );
    assert!(test.out.text().contains("  keys: 0\n  layers: none\n"));
    assert!(!test.out.text().contains("overwritten:"));
}
