mod support;

use envswitch::cli::{format_progress_event, version_line};
use envswitch::error::Error;
use envswitch::model::ProgressEvent;
use support::{FakeOp, FakeOpFactory, FakeVaultLister, ScriptedPrompter, TempProject};

#[test]
fn formats_plain_progress_output_without_ansi_colors() {
    assert_eq!(
        format_progress_event(
            &ProgressEvent::info("reading config item demo-envswitch"),
            false
        ),
        "envswitch [info] reading config item demo-envswitch"
    );
}

#[test]
fn formats_exact_sgr_bytes_for_every_status() {
    for (event, expected) in [
        (
            ProgressEvent::done("found 5 candidate item(s)"),
            "\x1b[2menvswitch\x1b[0m \x1b[32m[done]\x1b[0m found 5 candidate item(s)",
        ),
        (
            ProgressEvent::dry_run("would write apps/api/.env"),
            "\x1b[2menvswitch\x1b[0m \x1b[33m[dry-run]\x1b[0m would write apps/api/.env",
        ),
        (
            ProgressEvent::info("checking"),
            "\x1b[2menvswitch\x1b[0m \x1b[34m[info]\x1b[0m checking",
        ),
    ] {
        assert_eq!(
            format_progress_event(&event, true).as_bytes(),
            expected.as_bytes()
        );
    }
}

#[test]
fn validates_command_allowlists_and_boolean_values_before_work() {
    let project = TempProject::new();
    for (argv, message) in [
        (vec!["switch", "--unknown"], "unknown flag: --unknown"),
        (
            vec!["switch", "--dry-run=false"],
            "--dry-run does not take a value",
        ),
        (
            vec!["switch", "--dry-run", "foo"],
            "--dry-run does not take a value",
        ),
        (vec!["list", "--client", "alpha"], "unknown flag: --client"),
        (vec!["doctor", "--target", "all"], "unknown flag: --target"),
        (vec!["update", "--vault", "x"], "unknown flag: --vault"),
        (
            vec!["switch", "--__proto__", "x"],
            "unknown flag: --__proto__",
        ),
        (vec!["switch", "--z", "a", "--1", "b"], "unknown flag: --z"),
        (vec!["switch", "--"], "unknown flag: --"),
        (
            vec!["switch", "--help=false"],
            "--help does not take a value",
        ),
        (
            vec!["switch", "--help", "me"],
            "--help does not take a value",
        ),
        (vec!["switch", "-h"], "--vault is required"),
        (vec!["switch", "--vault"], "--vault requires a value"),
        (vec!["switch", "--dry-run", ""], "--vault is required"),
        (vec!["foo", "--help=1"], "unknown command: foo"),
        (vec![""], "unknown command: "),
        (vec!["--vault", "x", "switch"], "unknown command: --vault"),
        (vec!["-V"], "unknown command: -V"),
    ] {
        let mut test = support::deps(project.root());
        assert_eq!(test.run(&argv), 1, "{argv:?}");
        assert_eq!(
            test.err.text(),
            format!("envswitch: {message}\n"),
            "{argv:?}"
        );
        assert!(test.out.bytes().is_empty());
        assert!(test.factory.created().is_empty());
        assert!(test.installer.plans().is_empty());
    }
}

#[test]
fn every_help_path_has_verbatim_text_and_final_blank_line() {
    let project = TempProject::new();
    let mut test = support::deps(project.root());
    assert_eq!(test.run(&["--help"]), 0);
    let help = test.out.text();
    insta::assert_snapshot!(help, @r"
    envswitch

    Usage:
      envswitch
      envswitch switch --vault <vault> --config-item <item> --client <client> [--env local] [--target all|name] [--dry-run]
      envswitch list --vault <vault> --config-item <item> [clients|targets|layers]
      envswitch doctor --vault <vault> --config-item <item>
      envswitch update

    Notes:
      Run envswitch with no arguments for guided switch prompts.
      Run envswitch update to reinstall the latest GitHub release in place.
      Run envswitch from the root of the project you want to update.
      Project metadata, target paths, and tag rules are loaded from the 1Password config item.
    ");
    assert!(help.ends_with("item.\n\n"));
    for argv in [
        vec!["-h", "--bad"],
        vec!["--help", "me"],
        vec!["help", "--bad"],
        vec!["foo", "--help"],
        vec!["switch", "--unknown", "--help"],
        vec!["switch", "--help", ""],
        vec!["update", "--help"],
    ] {
        test.out.clear();
        assert_eq!(test.run(&argv), 0, "{argv:?}");
        assert_eq!(test.out.text(), help);
        assert!(test.err.bytes().is_empty());
    }
}

#[test]
fn version_is_stdout_only_and_preserves_the_s1_protocol() {
    let project = TempProject::new();
    let mut test = support::deps(project.root());
    assert_eq!(test.run(&["--version"]), 0);
    assert_eq!(test.out.text(), version_line());
    let normalized = test
        .out
        .text()
        .replace(env!("TARGET"), "{TARGET}")
        .replace(env!("CARGO_PKG_VERSION"), "{VERSION}");
    insta::assert_snapshot!(normalized, @"envswitch {VERSION} {TARGET}");
    assert!(test.err.bytes().is_empty());
    assert_eq!(test.run(&["switch", "--version"]), 1);
    assert_eq!(test.err.text(), "envswitch: unknown flag: --version\n");
}

#[test]
fn progress_color_obeys_stderr_ci_and_no_color_only() {
    let project = TempProject::new();
    for (tty, ci, no_color, colored) in [
        (true, None, None, true),
        (false, None, None, false),
        (true, Some(""), None, true),
        (true, Some("0"), None, false),
        (true, Some("false"), None, false),
        (true, None, Some(""), false),
        (true, None, Some("1"), false),
    ] {
        let prompt = ScriptedPrompter::new([]);
        let mut test = support::deps(project.root())
            .with_prompter(&prompt)
            .without_event_capture();
        test.deps.stderr_is_tty = tty;
        test.env.set("FORCE_COLOR", "1").set("CLICOLOR", "1");
        if let Some(ci) = ci {
            test.env.set("CI", ci);
        }
        if let Some(value) = no_color {
            test.env.set("NO_COLOR", value);
        }
        assert_eq!(test.run(&[]), 1);
        let prefix = if colored {
            "\x1b[2menvswitch\x1b[0m \x1b[34m[info]\x1b[0m listing 1Password vaults\n"
        } else {
            "envswitch [info] listing 1Password vaults\n"
        };
        assert!(test.err.text().starts_with(prefix));
        assert!(
            test.err
                .text()
                .ends_with("envswitch: no 1Password vaults found\n")
        );
    }
}

#[test]
fn error_taxonomy_prints_only_the_message_and_cancel_is_silent() {
    let project = TempProject::new();
    for error in [
        Error::user("invalid vaults"),
        Error::internal("op unavailable"),
        Error::Cancelled,
    ] {
        let prompt = ScriptedPrompter::new([]);
        let mut test = support::deps(project.root())
            .with_prompter(&prompt)
            .with_vaults(FakeVaultLister::failing(error.clone()));
        assert_eq!(test.run(&[]), error.exit_code());
        let expected = if error == Error::Cancelled {
            String::new()
        } else {
            format!("envswitch: {error}\n")
        };
        assert_eq!(test.err.text(), expected);
    }
    let op = FakeOp::new("v").with_config_error("cfg", Error::internal("private read failed"));
    // loadConfig emits the read progress event before a read failure, without reaching C1.
    let prompt = ScriptedPrompter::new([]);
    let op = op.with_items(vec![support::item(
        "cfg",
        "config title",
        &["envswitch-config"],
    )]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_vaults(FakeVaultLister::new(vec![support::vault("v", "V")]))
        .with_factory(FakeOpFactory::new().with(op));
    assert_eq!(test.run(&[]), 2);
    assert_eq!(test.err.text(), "envswitch: private read failed\n");
    assert!(
        test.events
            .lines()
            .contains(&"info:reading config item config title".into())
    );
}

#[test]
fn color_environment_is_rechecked_for_each_progress_event() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .without_event_capture();
    let lookups = std::cell::Cell::new(0);
    test.deps.stderr_is_tty = true;
    test.deps.env = Box::new(move |name| {
        if name != "CI" {
            return None;
        }
        let count = lookups.get();
        lookups.set(count + 1);
        if count == 0 { None } else { Some("1".into()) }
    });
    assert_eq!(test.run(&[]), 1);
    assert_eq!(
        test.err.text(),
        "\x1b[2menvswitch\x1b[0m \x1b[34m[info]\x1b[0m listing 1Password vaults\nenvswitch [done] found 0 vault(s)\nenvswitch: no 1Password vaults found\n"
    );
}

struct ClosedPipe;

impl std::io::Write for ClosedPipe {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn closed_stdout_does_not_fail_help_or_version() {
    let project = TempProject::new();
    for argv in [vec!["--help"], vec!["switch", "--help"], vec!["--version"]] {
        let mut test = support::deps(project.root());
        test.deps.out = Box::new(ClosedPipe);
        assert_eq!(test.run(&argv), 0, "{argv:?}");
        assert!(test.err.bytes().is_empty());
    }
}

#[test]
fn closed_stdout_does_not_replace_a_guided_exit_code() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_vaults(FakeVaultLister::failing(Error::Cancelled));
    test.deps.out = Box::new(ClosedPipe);
    assert_eq!(test.run(&[]), 130);
    assert_eq!(test.vaults.calls(), 1);
    assert!(test.err.bytes().is_empty());
}

#[test]
fn closed_stderr_does_not_replace_progress_or_error_exit_codes() {
    let project = TempProject::new();
    let prompt = ScriptedPrompter::new([]);
    let mut test = support::deps(project.root())
        .with_prompter(&prompt)
        .with_vaults(FakeVaultLister::failing(Error::Cancelled))
        .without_event_capture();
    test.deps.err = Box::new(ClosedPipe);
    assert_eq!(test.run(&[]), 130);
    assert_eq!(test.vaults.calls(), 1);
    assert_eq!(test.run(&["unknown"]), 1);
}
