//! S1 scaffold smoke tests: the crate links, S1-owned helpers behave, and the
//! test support compiles. Nothing here calls a worker stub (`todo!()`).

mod support;

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use envswitch::error::{Error, Result, fail, io_error, io_error_dest};
use envswitch::model::{LayerName, ProgressEvent, ProgressStatus};
use envswitch::ports::{
    Choice, InputSpec, OnePassword, OpFactory, Prompter, SelectSpec, VaultLister,
};

use support::{
    Answer, ConcurrencyGate, FakeOp, FakeOpFactory, OpCall, PromptRecord, ScriptedPrompter,
};

#[test]
fn version_line_has_version_and_target() {
    let line = envswitch::cli::version_line();
    let prefix = format!("envswitch {} ", env!("CARGO_PKG_VERSION"));
    assert!(line.starts_with(&prefix), "{line:?}");
    assert!(line.ends_with('\n') && !line.ends_with("\n\n"), "{line:?}");
    let triple = line[prefix.len()..].trim_end_matches('\n');
    assert!(
        triple.split('-').count() >= 3,
        "unexpected target triple {triple:?}"
    );
    assert!(triple.starts_with(std::env::consts::ARCH), "{triple:?}");
    assert!(!triple.contains(' '), "{triple:?}");
}

#[test]
fn error_exit_codes_and_display() {
    assert_eq!(Error::user("unknown command: x").exit_code(), 1);
    assert_eq!(Error::internal("spawn failed").exit_code(), 2);
    assert_eq!(Error::Cancelled.exit_code(), 130);
    assert_eq!(
        Error::user("unknown flag: --x").to_string(),
        "unknown flag: --x"
    );
    let failed: Result<()> = fail("--vault is required");
    assert_eq!(failed, Err(Error::User("--vault is required".to_string())));
}

#[test]
fn io_errors_use_node_format_for_listed_codes() {
    let path = Path::new("/root/afile/sub");
    let not_found = io::Error::from(io::ErrorKind::NotFound);
    assert_eq!(
        io_error(&not_found, "lstat", path),
        Error::Internal(format!(
            "ENOENT: no such file or directory, lstat '{}'",
            path.display()
        ))
    );
    let not_dir = io::Error::from(io::ErrorKind::NotADirectory);
    assert_eq!(
        io_error(&not_dir, "lstat", path).to_string(),
        format!("ENOTDIR: not a directory, lstat '{}'", path.display())
    );
    let is_dir = io::Error::from(io::ErrorKind::IsADirectory);
    let dest = Path::new("/root/.env");
    assert_eq!(
        io_error_dest(&is_dir, "rename", path, dest).to_string(),
        format!(
            "EISDIR: illegal operation on a directory, rename '{}' -> '{}'",
            path.display(),
            dest.display()
        )
    );
    let other = io::Error::other("disk on fire");
    assert_eq!(io_error(&other, "open", path).to_string(), "disk on fire");
    assert_eq!(io_error(&other, "open", path).exit_code(), 2);
}

#[cfg(unix)]
#[test]
fn io_errors_map_raw_errno() {
    let path = Path::new("/x");
    let eacces = io::Error::from_raw_os_error(13);
    assert_eq!(
        io_error(&eacces, "open", path).to_string(),
        "EACCES: permission denied, open '/x'"
    );
    let eperm = io::Error::from_raw_os_error(1);
    assert_eq!(
        io_error(&eperm, "chmod", path).to_string(),
        "EPERM: operation not permitted, chmod '/x'"
    );
    let eexist = io::Error::from_raw_os_error(17);
    assert_eq!(
        io_error(&eexist, "mkdir", path).to_string(),
        "EEXIST: file already exists, mkdir '/x'"
    );
}

#[test]
fn layer_names_and_statuses() {
    let names: Vec<&str> = LayerName::ALL.iter().map(|layer| layer.as_str()).collect();
    assert_eq!(
        names,
        ["shared", "group", "target", "client", "client-target"]
    );
    let ranks: Vec<u32> = LayerName::ALL.iter().map(|layer| layer.rank()).collect();
    assert_eq!(ranks, [1, 2, 3, 4, 5]);
    assert_eq!(ProgressStatus::DryRun.as_str(), "dry-run");
    assert_eq!(ProgressEvent::done("wrote x").status, ProgressStatus::Done);
}

#[test]
fn item_key_and_display_name_follow_ts_fallbacks() {
    let by_id = support::item("id1", "Title", &[]);
    assert_eq!((by_id.key(), by_id.display_name()), ("id1", "Title"));
    let no_id = support::item("", "Title", &[]);
    assert_eq!((no_id.key(), no_id.display_name()), ("Title", "Title"));
    let no_title = support::item("id1", "", &[]);
    assert_eq!((no_title.key(), no_title.display_name()), ("id1", "id1"));
}

#[test]
fn support_deps_build_without_touching_stubs() {
    let project = support::TempProject::new();
    project.dir("services").package_json("demo-suite");
    let t = support::deps(project.root());
    assert_eq!(t.deps.root, project.root());
    assert_eq!(t.deps.cwd, project.root());
    assert!(!t.deps.stdin_is_tty && !t.deps.stdout_is_tty && !t.deps.stderr_is_tty);
    assert!(t.deps.prompter.is_none());
    assert!((t.deps.env)("CI").is_none());
    t.env.set("NO_COLOR", "");
    assert_eq!((t.deps.env)("NO_COLOR"), Some("".into()));
    assert_eq!(project.entries(""), ["package.json", "services"]);
    assert_eq!(project.root().file_name().unwrap(), "envswitch-project");
}

#[test]
fn fake_op_records_calls_argv_and_read_counts() {
    let config_item = support::item("cfg", "demo config", &["envswitch-config"]);
    let env_item = support::item("i1", "shared", &["layer:shared"]);
    let op = FakeOp::new("vault-a")
        .with_list_for(&["envswitch-config"], Ok(vec![config_item.clone()]))
        .with_items(vec![env_item.clone()])
        .with_config("cfg", "{}")
        .with_values("i1", support::env_map(&[("A", "1")]));
    let factory = FakeOpFactory::new().with(op.clone());
    let client = factory.create("vault-a");

    assert_eq!(
        client
            .list_items_by_tags(&["envswitch-config".to_string()])
            .unwrap(),
        vec![config_item]
    );
    assert_eq!(
        client
            .list_items_by_tags(&["a".to_string(), "b".to_string()])
            .unwrap(),
        vec![env_item.clone()]
    );
    assert_eq!(client.read_config_item("cfg").unwrap(), "{}");
    assert!(client.read_config_item("missing").is_err());
    client.read_env_item(&env_item).unwrap();
    client.read_env_item(&env_item).unwrap();

    assert_eq!(factory.created(), ["vault-a"]);
    assert_eq!(op.read_counts().get("i1"), Some(&2));
    assert_eq!(
        op.calls()[0],
        OpCall::ListItemsByTags(vec!["envswitch-config".to_string()])
    );
    assert_eq!(
        op.argvs()[1],
        [
            "item", "list", "--vault", "vault-a", "--format", "json", "--tags", "a,b"
        ]
    );
    assert_eq!(
        op.argvs()[2],
        [
            "item", "get", "cfg", "--vault", "vault-a", "--format", "json", "--reveal"
        ]
    );
}

#[test]
fn concurrency_gate_passes_parallel_scans_and_times_out_sequential_ones() {
    let gate = ConcurrencyGate::new(2, Duration::from_secs(5));
    let a: Arc<dyn OnePassword> = Arc::new(FakeOp::new("a").with_gate(Arc::clone(&gate)));
    let b: Arc<dyn OnePassword> = Arc::new(FakeOp::new("b").with_gate(Arc::clone(&gate)));
    let results = std::thread::scope(|scope| {
        let handles = [&a, &b].map(|op| scope.spawn(move || op.list_items_by_tags(&[])));
        handles.map(|handle| handle.join().unwrap())
    });
    assert!(results.iter().all(|result| result.is_ok()));

    let lonely = FakeOp::new("c").with_gate(ConcurrencyGate::new(2, Duration::from_millis(20)));
    assert!(lonely.list_items_by_tags(&[]).is_err());
}

#[test]
fn scripted_prompter_records_and_answers() {
    let prompter = ScriptedPrompter::new([Answer::label("beta"), Answer::text("dev")]);
    let boxed: Box<dyn Prompter> = prompter.boxed();
    let select = SelectSpec {
        message: "Client".to_string(),
        choices: ["alpha", "beta"]
            .map(|label| Choice {
                label: label.to_string(),
            })
            .to_vec(),
        default_index: None,
    };
    assert_eq!(boxed.select(&select).unwrap(), 1);
    let input = InputSpec {
        message: "Environment".to_string(),
        default: Some("local".to_string()),
        required: true,
    };
    assert_eq!(boxed.input(&input).unwrap(), "dev");
    // Script exhausted: falls back to the default, like the TS FakePrompt.
    assert_eq!(boxed.input(&input).unwrap(), "local");
    assert_eq!(
        prompter.messages(),
        ["Client", "Environment", "Environment"]
    );
    assert_eq!(
        prompter.records()[0],
        PromptRecord::Select {
            message: "Client".to_string(),
            labels: vec!["alpha".into(), "beta".into()],
            default_index: None
        }
    );
    assert_eq!(
        ScriptedPrompter::cancelling().select(&select),
        Err(Error::Cancelled)
    );
}

#[test]
fn fake_vault_lister_counts_calls() {
    let lister = support::FakeVaultLister::new(vec![support::vault("vault-a", "Vault A")]);
    assert_eq!(lister.list_vaults().unwrap().len(), 1);
    assert_eq!(lister.calls(), 1);
}
