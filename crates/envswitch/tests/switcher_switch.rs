//! `switch_environment` and `inspect_project` through the public API
//! (contract 02 §4.3–4.6, §7; §8.2 rows 12–16). Owner: C3.
//!
//! Expected events, files, and results come from the TS oracle
//! (`src/switcher.ts` at `53de029`, Bun 1.4.0) run with the same config,
//! items, and values (`tests/switcher_oracle/scenarios.ts` prints them);
//! `{ROOT}` in the oracle output is the temp project root.
//! These tests need the real config (C1) and dotenv (C2) modules: they are
//! ignored until both merge, then run at the C3 merge gate.

mod support;

#[path = "../src/switcher/collation_golden.rs"]
#[allow(dead_code)]
mod collation_golden;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use envswitch::config::parse_config;
use envswitch::error::Error;
use envswitch::model::{Config, LayerName, ProgressEvent, SwitchRequest, SwitchResult};
use envswitch::switcher::{inspect_project, switch_environment};
use serde_json::json;

use support::{EventLog, FakeOp, OpCall, TempProject, env_map, item, link_dir};

fn ts_config() -> Config {
    parse_config(
        &json!({
            "project": { "id": "demo" },
            "clients": ["alpha", "beta"],
            "environments": ["local"],
            "targets": [
                { "name": "api", "output": "apps/api/.env", "groups": ["apis"] },
                { "name": "web", "output": "apps/web/.env", "groups": ["apps"] }
            ],
            "tags": { "base": ["envswitch"] }
        })
        .to_string(),
    )
    .expect("valid config")
}

const COMMON: [&str; 3] = ["envswitch", "project:demo", "env:local"];

fn tagged(id: &str, extra: &[&str]) -> envswitch::model::ItemSummary {
    let mut tags = COMMON.to_vec();
    tags.extend_from_slice(extra);
    item(id, id, &tags)
}

/// `switcher.test.ts` items and values, as a scripted [`FakeOp`].
fn ts_op() -> FakeOp {
    let items = vec![
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
    ];
    let mut op = FakeOp::new("Demo")
        .with_items(items)
        .with_values("shared", env_map(&[("SHARED", "1"), ("COMMON", "shared")]))
        .with_values(
            "api-group",
            env_map(&[("API_GROUP", "1"), ("COMMON", "api-group")]),
        )
        .with_values(
            "app-group",
            env_map(&[("APP_GROUP", "1"), ("COMMON", "app-group")]),
        )
        .with_values(
            "api-target",
            env_map(&[("API_ONLY", "1"), ("COMMON", "api")]),
        )
        .with_values(
            "web-target",
            env_map(&[("WEB_ONLY", "1"), ("COMMON", "web")]),
        )
        .with_values(
            "alpha",
            env_map(&[("CLIENT_ONLY", "1"), ("COMMON", "alpha")]),
        )
        .with_values(
            "alpha-web",
            env_map(&[("WEB_CLIENT_ONLY", "1"), ("COMMON", "alpha-web")]),
        )
        .with_values(
            "alpha-api",
            env_map(&[("API_CLIENT_ONLY", "1"), ("COMMON", "alpha-api")]),
        );
    for id in [
        "shared-with-group",
        "target-with-group",
        "target-with-client",
        "beta-web",
        "alpha-target-web",
        "shared-unknown-client",
        "shared-unknown-target",
        "shared-unknown-group",
    ] {
        op = op.with_values(id, env_map(&[("SHOULD_NOT_APPLY", "1")]));
    }
    op
}

fn request(config: Config, root: &Path, target: &str, dry_run: bool) -> SwitchRequest {
    SwitchRequest {
        config,
        root: root.to_path_buf(),
        cwd: root.to_path_buf(),
        env: "local".to_string(),
        client: "alpha".to_string(),
        target_name: target.to_string(),
        dry_run,
    }
}

/// Runs a switch and returns the result plus `"<status>:<message>"` lines
/// with the project root replaced by `{ROOT}` (like the oracle output).
fn run(op: &FakeOp, req: &SwitchRequest) -> (envswitch::error::Result<SwitchResult>, Vec<String>) {
    let log = EventLog::new();
    let result = switch_environment(op, req, &mut |event: ProgressEvent| log.push(event));
    let root = req.root.display().to_string();
    let lines = log
        .lines()
        .into_iter()
        .map(|line| line.replace(&root, "{ROOT}"))
        .collect();
    (result, lines)
}

/// `{ROOT}/apps/<name>/.env` with the platform separator, as the resolver
/// builds it.
fn out(name: &str) -> String {
    let path: PathBuf = ["{ROOT}", "apps", name, ".env"].iter().collect();
    path.display().to_string()
}

fn layer_summary(result: &SwitchResult, index: usize) -> Vec<String> {
    result.targets[index]
        .layers
        .iter()
        .map(|layer| format!("{}:{}", layer.name, layer.item.id))
        .collect()
}

// ---------------------------------------------------------------------------
// TS rows 12-16
// ---------------------------------------------------------------------------

#[test]
fn dry_run_merges_supported_layers_without_writing_files() {
    let project = TempProject::new();
    let op = ts_op();
    let (result, _) = run(&op, &request(ts_config(), project.root(), "api", true));
    let result = result.unwrap();
    assert_eq!(result.targets.len(), 1);
    let api = &result.targets[0];
    assert_eq!(api.target.name, "api");
    assert!(!api.written);
    assert_eq!(api.key_count, 6);
    assert_eq!(
        api.layers
            .iter()
            .map(|layer| layer.item.id.as_str())
            .collect::<Vec<_>>(),
        ["shared", "api-group", "api-target", "alpha", "alpha-api"]
    );
    assert_eq!(api.overwritten_keys, ["COMMON"]);
    assert_eq!(
        api.output_path,
        project.root().join("apps").join("api").join(".env")
    );
    assert!(!project.path("apps/api/.env").exists());
    assert!(!project.path("apps").exists());
}

#[test]
fn dry_run_logs_progress_when_a_logger_is_provided() {
    let project = TempProject::new();
    let (result, events) = run(&ts_op(), &request(ts_config(), project.root(), "api", true));
    result.unwrap();
    assert_eq!(
        events,
        [
            "info:validating project root {ROOT}".to_string(),
            "info:listing candidate 1Password items".to_string(),
            "done:found 16 candidate item(s)".to_string(),
            format!("info:processing api -> {}", out("api")),
            "info:reading shared".to_string(),
            "info:reading api-group".to_string(),
            "info:reading api-target".to_string(),
            "info:reading alpha".to_string(),
            "info:reading alpha-api".to_string(),
            format!("dry-run:would write {}", out("api")),
        ]
    );
}

#[test]
fn writes_merged_values_in_group_aware_layer_order() {
    let project = TempProject::new();
    let (result, _) = run(
        &ts_op(),
        &request(ts_config(), project.root(), "web", false),
    );
    let result = result.unwrap();
    assert_eq!(
        result.targets[0]
            .layers
            .iter()
            .map(|layer| layer.item.id.as_str())
            .collect::<Vec<_>>(),
        ["shared", "app-group", "web-target", "alpha", "alpha-web"]
    );
    assert!(result.targets[0].written);
    assert_eq!(
        project.read("apps/web/.env").unwrap(),
        "APP_GROUP=1\nCLIENT_ONLY=1\nCOMMON=alpha-web\nSHARED=1\nWEB_CLIENT_ONLY=1\nWEB_ONLY=1\n"
    );
}

#[test]
fn caches_item_reads_across_targets_in_one_switch() {
    let project = TempProject::new();
    let op = ts_op();
    let (result, events) = run(&op, &request(ts_config(), project.root(), "all", true));
    let result = result.unwrap();
    assert_eq!(
        result
            .targets
            .iter()
            .map(|target| target.target.name.as_str())
            .collect::<Vec<_>>(),
        ["api", "web"]
    );
    let counts = op.read_counts();
    // Reads overlap (DIVERGENCES l1), so the call order is not fixed; the set
    // and once-per-switch caching are. The "reading" events below keep the
    // oracle's sequential order.
    let mut read = counts.keys().map(String::as_str).collect::<Vec<_>>();
    read.sort_unstable();
    assert_eq!(
        read,
        [
            "alpha",
            "alpha-api",
            "alpha-web",
            "api-group",
            "api-target",
            "app-group",
            "shared",
            "web-target",
        ]
    );
    assert!(counts.values().all(|&count| count == 1), "{counts:?}");
    // Oracle: one list call with the candidate tags, then the reads.
    assert_eq!(
        op.tag_queries(),
        [vec!["envswitch", "project:demo", "env:local"]]
    );
    assert!(matches!(op.calls()[0], OpCall::ListItemsByTags(_)));
    assert!(
        op.calls()[1..]
            .iter()
            .all(|call| matches!(call, OpCall::ReadEnvItem(_))),
        "{:?}",
        op.calls()
    );
    // Oracle event sequence for `all` (dry run).
    assert_eq!(
        events,
        [
            "info:validating project root {ROOT}".to_string(),
            "info:listing candidate 1Password items".to_string(),
            "done:found 16 candidate item(s)".to_string(),
            format!("info:processing api -> {}", out("api")),
            "info:reading shared".to_string(),
            "info:reading api-group".to_string(),
            "info:reading api-target".to_string(),
            "info:reading alpha".to_string(),
            "info:reading alpha-api".to_string(),
            format!("dry-run:would write {}", out("api")),
            format!("info:processing web -> {}", out("web")),
            "info:reading app-group".to_string(),
            "info:reading web-target".to_string(),
            "info:reading alpha-web".to_string(),
            format!("dry-run:would write {}", out("web")),
        ]
    );
    assert_eq!(
        layer_summary(&result, 1),
        [
            "shared:shared",
            "group:app-group",
            "target:web-target",
            "client:alpha",
            "client-target:alpha-web",
        ]
    );
}

#[test]
fn rechecks_output_parents_immediately_before_writing() {
    let base = TempProject::named("project");
    let root = base.root().to_path_buf();
    let outside = base.outside().join("outside");
    let output_dir = root.join("apps").join("api");
    fs::create_dir_all(&output_dir).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let hook_dir = output_dir.clone();
    let hook_outside = outside.clone();
    let op = FakeOp::new("Demo")
        .with_items(vec![tagged("shared", &["layer:shared"])])
        .with_values("shared", env_map(&[("TOKEN", "secret")]))
        .on_read(move |_| {
            fs::remove_dir_all(&hook_dir).unwrap();
            link_dir(&hook_outside, &hook_dir).unwrap();
        });

    let (result, events) = run(&op, &request(ts_config(), &root, "api", false));
    let error = result.unwrap_err();
    assert_eq!(
        error,
        Error::user("target output path contains a symbolic-link parent: apps/api/.env")
    );
    assert!(!outside.join(".env").exists());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert_eq!(
        events.last().unwrap(),
        "info:reading shared",
        "nothing is logged after the failed re-check"
    );
}

// ---------------------------------------------------------------------------
// Write mode, failures, validation order (oracle scenarios)
// ---------------------------------------------------------------------------

#[test]
fn writes_every_target_with_oracle_events_and_bytes() {
    let project = TempProject::new();
    let (result, events) = run(
        &ts_op(),
        &request(ts_config(), project.root(), "all", false),
    );
    let result = result.unwrap();
    assert_eq!(
        events,
        [
            "info:validating project root {ROOT}".to_string(),
            "info:listing candidate 1Password items".to_string(),
            "done:found 16 candidate item(s)".to_string(),
            format!("info:processing api -> {}", out("api")),
            "info:reading shared".to_string(),
            "info:reading api-group".to_string(),
            "info:reading api-target".to_string(),
            "info:reading alpha".to_string(),
            "info:reading alpha-api".to_string(),
            format!("info:writing {}", out("api")),
            format!("done:wrote {}", out("api")),
            format!("info:processing web -> {}", out("web")),
            "info:reading app-group".to_string(),
            "info:reading web-target".to_string(),
            "info:reading alpha-web".to_string(),
            format!("info:writing {}", out("web")),
            format!("done:wrote {}", out("web")),
        ]
    );
    for (index, name, keys) in [(0, "api", 6), (1, "web", 6)] {
        let target = &result.targets[index];
        assert_eq!(target.target.name, name);
        assert!(target.written);
        assert_eq!(target.key_count, keys);
        assert_eq!(target.overwritten_keys, ["COMMON"]);
    }
    assert_eq!(
        project.read("apps/api/.env").unwrap(),
        "API_CLIENT_ONLY=1\nAPI_GROUP=1\nAPI_ONLY=1\nCLIENT_ONLY=1\nCOMMON=alpha-api\nSHARED=1\n"
    );
    assert_eq!(
        project.read("apps/web/.env").unwrap(),
        "APP_GROUP=1\nCLIENT_ONLY=1\nCOMMON=alpha-web\nSHARED=1\nWEB_CLIENT_ONLY=1\nWEB_ONLY=1\n"
    );
    assert_eq!(project.entries("apps/api"), [".env"]);
}

#[test]
fn a_failed_read_aborts_after_earlier_targets_were_written() {
    let project = TempProject::new();
    let op = ts_op().with_read_error("app-group", Error::internal("op read failed"));
    let (result, events) = run(&op, &request(ts_config(), project.root(), "all", false));
    assert_eq!(result.unwrap_err(), Error::internal("op read failed"));
    assert_eq!(
        events,
        [
            "info:validating project root {ROOT}".to_string(),
            "info:listing candidate 1Password items".to_string(),
            "done:found 16 candidate item(s)".to_string(),
            format!("info:processing api -> {}", out("api")),
            "info:reading shared".to_string(),
            "info:reading api-group".to_string(),
            "info:reading api-target".to_string(),
            "info:reading alpha".to_string(),
            "info:reading alpha-api".to_string(),
            format!("info:writing {}", out("api")),
            format!("done:wrote {}", out("api")),
            format!("info:processing web -> {}", out("web")),
            "info:reading app-group".to_string(),
        ]
    );
    assert!(project.path("apps/api/.env").exists());
    assert!(!project.path("apps/web/.env").exists());
}

#[test]
fn the_earliest_failing_read_wins_even_when_later_reads_overlap() {
    // DIVERGENCES l1: reads overlap, but results are consumed in TS order, so
    // the first failing item in that order is reported and nothing is written.
    for _ in 0..25 {
        let project = TempProject::new();
        let op = ts_op()
            .with_read_error("alpha-web", Error::internal("later read failed"))
            .with_read_error("api-target", Error::internal("earlier read failed"));
        let (result, events) = run(&op, &request(ts_config(), project.root(), "all", false));
        assert_eq!(result.unwrap_err(), Error::internal("earlier read failed"));
        assert_eq!(
            events,
            [
                "info:validating project root {ROOT}".to_string(),
                "info:listing candidate 1Password items".to_string(),
                "done:found 16 candidate item(s)".to_string(),
                format!("info:processing api -> {}", out("api")),
                "info:reading shared".to_string(),
                "info:reading api-group".to_string(),
                "info:reading api-target".to_string(),
            ]
        );
        assert!(!project.path("apps/api/.env").exists());
        assert!(!project.path("apps/web/.env").exists());
    }
}

#[test]
fn validation_fails_before_any_op_call() {
    // Oracle: only `validating project root` is logged and `op` is never
    // called when the root, selection, or an output path is invalid.
    let cases = [
        (
            json!({ "targets": [{ "name": "api", "output": "../x/.env" }] }),
            "local",
            "api",
            "target output path escapes the project root: ../x/.env",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": ".env" }] }),
            "bad env",
            "api",
            "environment contains unsupported characters",
        ),
        (
            json!({ "targets": [{ "name": "api", "output": ".env" }] }),
            "local",
            "worker",
            "target worker is not defined in config.targets",
        ),
        (
            json!({
                "project": { "rootMarkers": [{ "path": "services" }] },
                "targets": [{ "name": "api", "output": ".env" }]
            }),
            "local",
            "api",
            "project root marker path does not exist: services",
        ),
    ];
    for (config, env, target, message) in cases {
        let project = TempProject::new();
        let op = FakeOp::new("Demo");
        let mut req = request(
            parse_config(&config.to_string()).unwrap(),
            project.root(),
            target,
            true,
        );
        req.env = env.to_string();
        let (result, events) = run(&op, &req);
        assert_eq!(result.unwrap_err(), Error::user(message));
        assert_eq!(events, ["info:validating project root {ROOT}"]);
        assert!(op.calls().is_empty(), "{message}");
    }
}

#[test]
fn an_empty_candidate_list_writes_a_single_newline() {
    let project = TempProject::new();
    let op = FakeOp::new("Demo");
    let (result, events) = run(&op, &request(ts_config(), project.root(), "api", false));
    assert_eq!(result.unwrap().targets[0].key_count, 0);
    assert_eq!(
        events,
        [
            "info:validating project root {ROOT}".to_string(),
            "info:listing candidate 1Password items".to_string(),
            "done:found 0 candidate item(s)".to_string(),
            format!("info:processing api -> {}", out("api")),
            format!("info:writing {}", out("api")),
            format!("done:wrote {}", out("api")),
        ]
    );
    assert_eq!(project.read("apps/api/.env").unwrap(), "\n");
}

#[test]
fn inspect_reports_output_paths_and_parent_existence() {
    let project = TempProject::new();
    project.dir("apps/api");
    let report = inspect_project(&ts_config(), project.root()).unwrap();
    let rows: Vec<(String, PathBuf, bool)> = report
        .targets
        .into_iter()
        .map(|target| (target.target.name, target.output_path, target.parent_exists))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "api".to_string(),
                project.root().join("apps").join("api").join(".env"),
                true
            ),
            (
                "web".to_string(),
                project.root().join("apps").join("web").join(".env"),
                false
            ),
        ]
    );
}

#[test]
fn inspect_checks_root_markers_first() {
    let project = TempProject::new();
    let config = parse_config(
        &json!({
            "project": { "rootMarkers": [{ "path": "services" }] },
            "targets": [{ "name": "api", "output": "../escape/.env" }]
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        inspect_project(&config, project.root()).unwrap_err(),
        Error::user("project root marker path does not exist: services")
    );
}

// ---------------------------------------------------------------------------
// Collation end to end (D8)
// ---------------------------------------------------------------------------

#[test]
fn collation_golden_writes_the_oracle_env_bytes() {
    use collation_golden::{FULL_ENV, GROUPS, TITLES};

    let project = TempProject::new();
    let config = parse_config(
        &json!({
            "project": { "id": "demo" },
            "targets": [{ "name": "api", "output": ".env" }],
            "tags": { "base": ["envswitch"] }
        })
        .to_string(),
    )
    .unwrap();
    let tags = ["envswitch", "project:demo", "env:local", "layer:shared"];
    let mut op = FakeOp::new("Demo");
    let mut items = Vec::new();
    for (index, title) in TITLES.iter().enumerate() {
        let id = format!("t{index:03}");
        items.push(item(&id, title, &tags));
        op = op.with_values(&id, env_map(&[("COMMON", &id)]));
    }
    let op = op.with_items(items);

    let (result, _) = run(&op, &request(config, project.root(), "api", false));
    let result = result.unwrap();
    let expected: Vec<String> = GROUPS
        .iter()
        .flat_map(|group| {
            let mut members = group.to_vec();
            members.sort_unstable();
            members.into_iter().map(|index| format!("t{index:03}"))
        })
        .collect();
    assert_eq!(
        result.targets[0]
            .layers
            .iter()
            .map(|layer| layer.item.id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        result.targets[0]
            .layers
            .iter()
            .all(|layer| layer.name == LayerName::Shared)
    );
    assert_eq!(project.read(".env").unwrap(), FULL_ENV);
}

// ---------------------------------------------------------------------------
// The library never prints (TS row 12 checks `console.error`)
// ---------------------------------------------------------------------------

const CHILD_ENV: &str = "ENVSWITCH_C3_SWITCH_CHILD";

#[test]
fn switch_and_inspect_never_print() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let project = TempProject::new();
        for (target, dry_run) in [("all", true), ("all", false)] {
            let req = request(ts_config(), project.root(), target, dry_run);
            switch_environment(&ts_op(), &req, &mut |_| {}).unwrap();
        }
        let failing = ts_op().with_read_error("alpha", Error::internal("op read failed"));
        let req = request(ts_config(), project.root(), "api", true);
        switch_environment(&failing, &req, &mut |_| {}).unwrap_err();
        inspect_project(&ts_config(), project.root()).unwrap();
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "switch_and_inspect_never_print",
            "--nocapture",
            "--include-ignored",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
    assert!(stderr.is_empty(), "library wrote to stderr: {stderr:?}");
    for line in stdout.lines() {
        assert!(
            line.is_empty()
                || line.starts_with("running ")
                || line == "test switch_and_inspect_never_print ... ok"
                || line.starts_with("test result: "),
            "library wrote to stdout: {line:?}"
        );
    }
}
