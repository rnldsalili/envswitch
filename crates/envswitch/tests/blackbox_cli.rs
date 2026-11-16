#[path = "support/process.rs"]
mod process;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use process::Sandbox;
use serde_json::{Value, json};

fn fixture() -> Value {
    let config = json!({"project":{"id":"demo"},"clients":["alpha","beta"],
        "environments":["local","dev"],"targets":[{"name":"api","output":".env"}],"tags":{"base":["envswitch"]}});
    let cfg = json!({"id":"cfg","title":"Project config","tags":["envswitch-config"]});
    let item = json!({"id":"shared","title":"Shared","tags":["envswitch","project:demo","env:local","layer:shared"]});
    let response = |argv: &[&str], stdout: Value| json!({"argv":argv,"stdout":stdout.to_string()});
    json!({"responses":[
        response(&["vault","list","--format","json"], json!([{"id":"vault","name":"Demo"}])),
        response(&["item","list","--vault","vault","--format","json","--tags","envswitch-config"], json!([cfg])),
        response(&["item","get","cfg","--vault","vault","--format","json","--reveal"], json!({"fields":[{"label":"config","value":config.to_string()}]})),
        response(&["item","list","--vault","vault","--format","json","--tags","envswitch,project:demo,env:local"], json!([item])),
        response(&["item","get","shared","--vault","vault","--format","json","--reveal"], json!({"fields":[{"label":"Z","value":"last"},{"label":"A","value":"hello world"}]})),
        response(&["whoami","--format","json"], json!({"id":"test"})),
    ]})
}

fn project() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.fake_op(&fixture());
    sandbox.write(
        &sandbox.project.join("package.json"),
        b"{\"name\":\"demo\"}",
    );
    sandbox
}

const CONFIG_PROGRESS: &str =
    "envswitch [info] reading config item cfg\nenvswitch [done] loaded config item cfg\n";

#[test]
fn fake_op_fixture_delay_is_applied_and_invalid_delays_fail_closed() {
    for delay in [json!(60), json!(-1)] {
        let s = Sandbox::new();
        s.fake_op(&json!({"responses":[{"argv":["whoami","--format","json"],
            "stdout":"{}","delayMs":delay}]}));
        let start = std::time::Instant::now();
        let output = s
            .op_command(s.tools.join(if cfg!(windows) { "op.exe" } else { "op" }))
            .args(["whoami", "--format", "json"])
            .output()
            .unwrap();
        if delay == json!(60) {
            assert!(start.elapsed() >= std::time::Duration::from_millis(60));
            assert!(output.status.success());
            assert_eq!(output.stdout, b"{}");
            assert!(output.stderr.is_empty());
        } else {
            assert_eq!(output.status.code(), Some(97));
            assert!(output.stdout.is_empty());
            assert_eq!(
                output.stderr,
                b"fake-op: fixture delayMs must be an unsigned integer\n"
            );
        }
    }
}

#[test]
fn switch_dry_run_has_exact_output_and_does_not_write() {
    let s = project();
    let root = s.project.display();
    let output = s.project.join(".env");
    let stdout = format!(
        "Dry run 1 target(s) from project\n- api -> {}\n  keys: 2\n  layers: shared:Shared\n",
        output.display()
    );
    let stderr = format!(
        "{CONFIG_PROGRESS}envswitch [info] validating project root {root}\nenvswitch [info] listing candidate 1Password items\nenvswitch [done] found 1 candidate item(s)\nenvswitch [info] processing api -> {}\nenvswitch [info] reading Shared\nenvswitch [dry-run] would write {}\n",
        output.display(),
        output.display()
    );
    s.cli()
        .args([
            "switch",
            "--vault",
            "vault",
            "--config-item",
            "cfg",
            "--client",
            "alpha",
            "--dry-run",
        ])
        .assert()
        .code(0)
        .stdout(predicate::eq(stdout))
        .stderr(predicate::eq(stderr));
    assert!(!output.exists());
}

#[test]
fn list_targets_has_exact_output() {
    let s = project();
    s.cli()
        .args([
            "list",
            "targets",
            "--vault",
            "vault",
            "--config-item",
            "cfg",
        ])
        .assert()
        .code(0)
        .stdout(predicate::eq("api\t.env\n"))
        .stderr(predicate::eq(CONFIG_PROGRESS));
}

#[test]
fn doctor_has_exact_output() {
    let s = project();
    let stdout = format!(
        "1Password authentication: ok\nproject root: {}\ntargets: 1\n- api -> {} (ok)\n",
        s.project.display(),
        s.project.join(".env").display()
    );
    let stderr = format!(
        "envswitch [info] checking 1Password authentication\nenvswitch [done] 1Password authentication ok\n{CONFIG_PROGRESS}envswitch [info] inspecting project root {}\n",
        s.project.display()
    );
    s.cli()
        .args(["doctor", "--vault", "vault", "--config-item", "cfg"])
        .assert()
        .code(0)
        .stdout(predicate::eq(stdout))
        .stderr(predicate::eq(stderr));
}

#[cfg(target_os = "linux")]
#[test]
fn switch_from_non_utf8_cwd_writes_to_the_real_root() {
    use std::{ffi::OsStr, fs, os::unix::ffi::OsStrExt};

    let s = project();
    let root = s.root.join(OsStr::from_bytes(b"proj\xff"));
    let lossy_sibling = s.root.join("proj\u{fffd}");
    for dir in [&root, &lossy_sibling] {
        s.owned(dir);
        fs::create_dir(dir).unwrap();
    }
    let output = root.join(".env");
    s.owned(&output);
    s.write(&lossy_sibling.join(".env"), b"untouched\n");
    s.write(&root.join("package.json"), b"{\"name\":\"demo\"}");
    s.cli().current_dir(&root).args([
        "switch", "--vault", "vault", "--config-item", "cfg", "--client", "alpha",
    ]).assert().code(0)
        .stdout(predicate::eq(format!("Updated 1 target(s) from proj\u{fffd}\n- api -> {}\n  keys: 2\n  layers: shared:Shared\n", output.display())))
        .stderr(predicate::str::contains("envswitch:").not());
    assert_eq!(fs::read(&output).unwrap(), b"A=\"hello world\"\nZ=last\n");
    assert_eq!(
        fs::read(lossy_sibling.join(".env")).unwrap(),
        b"untouched\n"
    );
}

#[cfg(unix)]
mod pty {
    use super::*;
    use expectrl::{Eof, Expect, Session, process::unix::WaitStatus};
    use std::time::Duration;

    fn prompt(session: &mut expectrl::session::OsSession, message: &str) {
        session.expect(message).unwrap();
    }

    #[test]
    fn real_guided_choices_write_expected_env_bytes() {
        let s = project();
        let output = s.project.join(".env");
        s.owned(&output);
        let mut session = Session::spawn(s.cli()).unwrap();
        session.set_expect_timeout(Some(Duration::from_secs(10)));
        prompt(&mut session, "Client");
        session.send("beta\r").unwrap();
        prompt(&mut session, "Environment");
        session.send("\r").unwrap();
        prompt(&mut session, "Mode");
        session.send("\r").unwrap();
        session.expect("Updated 1 target(s) from project").unwrap();
        session.expect(Eof).unwrap();
        assert!(matches!(
            session.get_process().wait().unwrap(),
            WaitStatus::Exited(_, 0)
        ));
        assert_eq!(
            std::fs::read(output).unwrap(),
            b"A=\"hello world\"\nZ=last\n"
        );
    }

    #[test]
    fn real_guided_escape_is_130_without_a_fatal_line_or_write() {
        let s = project();
        let mut session = Session::spawn(s.cli()).unwrap();
        session.set_expect_timeout(Some(Duration::from_secs(10)));
        prompt(&mut session, "Client");
        session.send("\x1b").unwrap();
        let tail = session.expect(Eof).unwrap();
        assert!(!String::from_utf8_lossy(tail.as_bytes()).contains("envswitch:"));
        assert!(matches!(
            session.get_process().wait().unwrap(),
            WaitStatus::Exited(_, 130)
        ));
        assert!(!s.project.join(".env").exists());
    }
}
