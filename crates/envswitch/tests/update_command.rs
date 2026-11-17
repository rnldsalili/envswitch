//! `update` against a fake installed layout.
//!
//! `installed()` names the payload after `env!("TARGET")`, and receipts only
//! accept triples the release pipeline ships. On any other triple (such as
//! the Linux host default `x86_64-unknown-linux-gnu`) the product rejects
//! every receipt by design, so tests built on that fixture would either fail
//! or pass vacuously. They are ignored there with a reason; CI runs the Linux
//! suite on `x86_64-unknown-linux-musl`, the triple that ships.
mod support;

use std::fs;
use support::{FakeInstaller, TestDeps};
use tempfile::TempDir;

const RECEIPT_ERROR: &str = "envswitch: cannot locate a valid envswitch install receipt for this binary; reinstall envswitch with the install script from the README\n";

fn physical(path: &std::path::Path) -> PathBuf {
    let p = path.canonicalize().unwrap();
    #[cfg(windows)]
    if let Some(s) = p.to_str().unwrap().strip_prefix("\\\\?\\") {
        return PathBuf::from(s);
    }
    p
}

/// Assertion context for update runs: stderr plus the receipt next to
/// `current_exe` and whether the product accepts it, so a CI failure shows
/// why the receipt was rejected instead of only the generic guidance.
fn diag(t: &TestDeps) -> String {
    let exe = t.deps.current_exe.clone();
    let root = exe
        .as_deref()
        .and_then(|e| Some(e.parent()?.parent()?.parent()?.to_path_buf()));
    let receipt = root.as_ref().map(|r| {
        fs::read_to_string(r.join("current").join("install.json"))
            .unwrap_or_else(|e| format!("<unreadable: {e}>"))
    });
    let accepted = root
        .as_ref()
        .map(|r| envswitch::install::receipt::read_receipt(r, &t.deps.env).is_some());
    format!(
        "stderr: {}\ncurrent_exe: {exe:?}\ntarget: {}\nreceipt: {receipt:?}\nread_receipt accepted: {accepted:?}",
        t.err.text(),
        env!("TARGET")
    )
}

fn installed() -> (TempDir, TestDeps) {
    let temp = tempfile::tempdir().unwrap();
    let root = physical(temp.path()).join("app");
    let current = root.join("current");
    let bin = physical(temp.path()).join("launchers");
    fs::create_dir_all(current.join("bin")).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let payload = current.join("bin").join(format!(
        "envswitch_{}_{}{suffix}",
        env!("CARGO_PKG_VERSION"),
        env!("TARGET")
    ));
    fs::write(&payload, b"binary").unwrap();
    let receipt = serde_json::json!({"schemaVersion":1,"kind":"native","repo":"demo/suite",
        "appRoot":root,"installDir":bin,"launcherPath":bin.join(if cfg!(windows) {"envswitch.cmd"} else {"envswitch"}),
        "target":env!("TARGET"),"tag":format!("v{}",env!("CARGO_PKG_VERSION")),"payloadPath":payload});
    fs::write(current.join("install.json"), receipt.to_string()).unwrap();
    let mut t = support::deps(temp.path());
    t.deps.current_exe = Some(payload);
    (temp, t)
}

#[test]
fn copied_binary_requires_reinstall_and_never_calls_installer() {
    let temp = tempfile::tempdir().unwrap();
    let mut t = support::deps(temp.path());
    let exe = temp.path().join("envswitch");
    fs::write(&exe, b"binary").unwrap();
    t.deps.current_exe = Some(exe);
    assert_eq!(t.run(&["update"]), 1);
    assert_eq!(t.err.text(), RECEIPT_ERROR);
    assert!(t.installer.plans().is_empty());
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn child_status_is_forwarded_and_latest_is_not_resolved_by_rust() {
    let (_temp, mut t) = installed();
    t = t.with_installer(FakeInstaller::new().with_result(Ok(143)));
    assert_eq!(t.run(&["update"]), 143, "{}", diag(&t));
    assert_eq!(
        t.err.text(),
        "envswitch: update failed: installer exited with code 143\n"
    );
    assert!(t.out.text().is_empty());
    let plans = t.installer.plans();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].repo, "demo/suite");
    assert_eq!(plans[0].version, None);
    assert_eq!(
        plans[0].current_version,
        format!("v{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn zero_status_without_result_is_not_success() {
    let (_temp, mut t) = installed();
    assert_eq!(t.run(&["update"]), 1);
    assert!(
        t.err.text().contains("installer result does not match"),
        "{}",
        diag(&t)
    );
    assert!(t.out.text().is_empty());
}

use envswitch::error::{Error, Result};
use envswitch::ports::{InstallPlan, InstallerRunner};
use std::io::Write;
use std::path::PathBuf;

struct TranscriptInstaller {
    fake: FakeInstaller,
    stdout: String,
}
impl InstallerRunner for TranscriptInstaller {
    fn run(&self, plan: &InstallPlan) -> Result<i32> {
        self.fake.run(plan)
    }
    fn run_with_output(&self, plan: &InstallPlan, out: &mut dyn Write) -> Result<(i32, String)> {
        let code = self.fake.run(plan)?;
        out.write_all(self.stdout.as_bytes()).unwrap();
        Ok((code, self.stdout.clone()))
    }
}

fn current(t: &TestDeps) -> PathBuf {
    t.deps
        .current_exe
        .as_ref()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn result_fixture(t: &mut TestDeps, tag: &str, outcome: &str) -> String {
    let current = current(t);
    let mut r: serde_json::Value =
        serde_json::from_slice(&fs::read(current.join("install.json")).unwrap()).unwrap();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let payload = current.join("bin").join(format!(
        "envswitch_{}_{}{suffix}",
        &tag[1..],
        env!("TARGET")
    ));
    r["tag"] = tag.into();
    r["payloadPath"] = payload.to_str().unwrap().into();
    let launcher = PathBuf::from(r["launcherPath"].as_str().unwrap());
    let dir = r["installDir"].as_str().unwrap();
    let terminal = if outcome == "unchanged" {
        format!("success: envswitch is already up to date ({tag})\n")
    } else {
        let guidance = if cfg!(windows) {
            format!(
                "info: restart your terminal, or run this command now:\ninfo: $env:Path = \"{dir};$env:Path\"\ninfo: then run 'envswitch --help' to verify the installation\n"
            )
        } else {
            "info: run 'envswitch --help' to verify the installation\n".to_owned()
        };
        format!(
            "success: {outcome} envswitch {tag}\ninfo: binary: {}\ninfo: launcher: {}\n{guidance}",
            payload.display(),
            launcher.display()
        )
    };
    let marker = format!("{tag}\n");
    let fake = FakeInstaller::new().on_run(move |_| {
        fs::write(&payload, b"new binary").unwrap();
        fs::write(&launcher, b"launcher").unwrap();
        fs::write(current.join("install.json"), r.to_string()).unwrap();
        fs::write(current.join(".version"), &marker).unwrap();
    });
    t.installer = fake.clone();
    t.deps.installer = Box::new(TranscriptInstaller {
        fake,
        stdout: terminal.clone(),
    });
    terminal
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn equal_version_still_invokes_installer_and_reports_unchanged_or_repaired() {
    for (outcome, expected) in [
        ("unchanged", "envswitch is already up to date (v0.0.0)\n"),
        ("repaired", "repaired envswitch v0.0.0\n"),
    ] {
        let (_temp, mut t) = installed();
        // A missing marker or bundled script must not shortcut repair/health work.
        let transcript = result_fixture(&mut t, "v0.0.0", outcome);
        assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
        assert_eq!(t.out.text(), format!("{transcript}{expected}"));
        assert_eq!(t.installer.plans().len(), 1);
    }
}

#[cfg(windows)]
#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn decoded_installer_location_warning_uses_injected_stderr() {
    let (_temp, mut t) = installed();
    let transcript = result_fixture(&mut t, "v0.0.42", "installed");
    let stdout = transcript
        .lines()
        .map(|line| {
            if line.starts_with("info: binary: ") || line.starts_with("info: launcher: ") {
                format!("{}: decoded?", line.rsplit_once(": ").unwrap().0)
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    t.deps.installer = Box::new(TranscriptInstaller {
        fake: t.installer.clone(),
        stdout,
    });
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
    assert_eq!(
        t.err.text(),
        "envswitch: warning: installer locations could not be matched after console decoding; validated the installed receipt instead\n"
    );
    assert!(
        t.out
            .text()
            .ends_with("updated envswitch v0.0.0 -> v0.0.42\n")
    );
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn compiled_version_labels_update_even_when_marker_is_missing_or_stale() {
    for marker in [None, Some("v0.0.999\n")] {
        let (_temp, mut t) = installed();
        if let Some(marker) = marker {
            fs::write(current(&t).join(".version"), marker).unwrap();
        }
        let transcript = result_fixture(&mut t, "v0.0.42", "installed");
        t.deps.log = None;
        assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
        assert_eq!(
            t.out.text(),
            format!("{transcript}updated envswitch v0.0.0 -> v0.0.42\n")
        );
        assert_eq!(
            t.err.text(),
            "envswitch [info] updating envswitch v0.0.0 -> latest\n"
        );
    }
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn racing_old_process_reports_updated_when_installer_returns_unchanged() {
    let (_temp, mut t) = installed();
    // Persisted settings may already describe a newer process's installation.
    let rpath = current(&t).join("install.json");
    let mut r: serde_json::Value = serde_json::from_slice(&fs::read(&rpath).unwrap()).unwrap();
    r["tag"] = "v0.0.42".into();
    r["payloadPath"] = current(&t)
        .join("bin")
        .join(format!(
            "envswitch_0.0.42_{}{}",
            env!("TARGET"),
            if cfg!(windows) { ".exe" } else { "" }
        ))
        .to_str()
        .unwrap()
        .into();
    fs::write(rpath, r.to_string()).unwrap();
    let transcript = result_fixture(&mut t, "v0.0.42", "unchanged");
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
    assert_eq!(
        t.out.text(),
        format!("{transcript}updated envswitch v0.0.0 -> v0.0.42\n")
    );
}

#[test]
fn rejects_checkout_and_cargo_target_before_installation() {
    for git_dir in [false, true] {
        let (_temp, mut t) = installed();
        let app = current(&t);
        if git_dir {
            fs::create_dir(app.join(".git")).unwrap();
        } else {
            fs::write(app.join(".git"), b"gitdir: elsewhere").unwrap();
        }
        assert_eq!(t.run(&["update"]), 1);
        assert!(
            t.err
                .text()
                .contains("looks like a source checkout (use git pull instead)")
        );
        assert!(t.installer.plans().is_empty());
    }
    for relative in [
        "target/debug/envswitch",
        "target/aarch64-apple-darwin/release/envswitch",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"binary").unwrap();
        let mut t = support::deps(temp.path());
        t.deps.current_exe = Some(path);
        assert_eq!(t.run(&["update"]), 1);
        assert!(t.err.text().contains("looks like a source checkout"));
    }
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn invoking_project_git_is_not_used_as_installation_anchor() {
    let (temp, mut t) = installed();
    fs::create_dir(temp.path().join(".git")).unwrap();
    result_fixture(&mut t, "v0.0.0", "repaired");
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn app_dir_override_guards_source_checkout_and_validates_installation_root() {
    let (temp, mut t) = installed();
    let source = temp.path().join("checkout");
    fs::create_dir_all(source.join(".git")).unwrap();
    t.deps.app_dir = Some(source);
    assert_eq!(t.run(&["update"]), 1);
    assert!(t.err.text().contains("looks like a source checkout"));
    t.deps.app_dir = Some(current(&t));
    t.err.clear();
    result_fixture(&mut t, "v0.0.0", "unchanged");
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn invalid_receipts_never_authorize_installer() {
    for (field, value) in [
        ("schemaVersion", serde_json::json!(2)),
        ("schemaVersion", serde_json::json!(1.0)),
        ("kind", serde_json::json!("other")),
        ("repo", serde_json::json!("../bad")),
        ("tag", serde_json::json!("v0.0.01")),
        ("tag", serde_json::json!("0.0.42")),
        ("target", serde_json::json!(null)),
        ("payloadPath", serde_json::json!("/other/file")),
        ("launcherPath", serde_json::json!("/other/launcher")),
        ("appRoot", serde_json::json!("/other/root")),
        ("installDir", serde_json::json!("relative")),
    ] {
        let (_temp, mut t) = installed();
        let path = current(&t).join("install.json");
        let mut r: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        r[field] = value;
        fs::write(path, r.to_string()).unwrap();
        assert_eq!(t.run(&["update"]), 1, "{field}");
        assert_eq!(t.err.text(), RECEIPT_ERROR, "{field}");
        assert!(t.installer.plans().is_empty());
    }
    for contents in [
        "{}",
        "not json",
        "{\"schemaVersion\":1,\"schemaVersion\":1}",
    ] {
        let (_temp, mut t) = installed();
        fs::write(current(&t).join("install.json"), contents).unwrap();
        assert_eq!(t.run(&["update"]), 1);
        assert_eq!(t.err.text(), RECEIPT_ERROR);
    }
    let (_temp, mut t) = installed();
    let path = current(&t).join("install.json");
    let bytes = fs::read_to_string(&path).unwrap();
    fs::write(&path, bytes.replacen('{', "{\"schemaVersion\":1,", 1)).unwrap();
    assert_eq!(t.run(&["update"]), 1);
    assert_eq!(t.err.text(), RECEIPT_ERROR);
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn missing_receipt_is_reinstall_guidance_and_unknown_flag_precedes_lookup() {
    let (_temp, mut t) = installed();
    fs::remove_file(current(&t).join("install.json")).unwrap();
    assert_eq!(t.run(&["update", "--vault", "x"]), 1);
    assert_eq!(t.err.text(), "envswitch: unknown flag: --vault\n");
    t.err.clear();
    assert_eq!(t.run(&["update"]), 1);
    assert_eq!(t.err.text(), RECEIPT_ERROR);
    assert!(t.installer.plans().is_empty());
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn spawn_failure_remains_internal_and_prints_no_success() {
    let (_temp, mut t) = installed();
    t = t.with_installer(FakeInstaller::new().with_result(Err(Error::internal(
        "update failed: could not run installer: missing bash",
    ))));
    assert_eq!(t.run(&["update"]), 2, "{}", diag(&t));
    assert_eq!(
        t.err.text(),
        "envswitch: update failed: could not run installer: missing bash\n"
    );
    assert!(t.out.text().is_empty());
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn inconsistent_postconditions_or_forged_tool_output_never_print_cli_success() {
    for fault in [
        "marker",
        "payload",
        "launcher",
        "repo",
        "installDir",
        "terminal-tag",
        "suffix",
        "no-newline",
    ] {
        let (_temp, mut t) = installed();
        let mut transcript = result_fixture(&mut t, "v0.0.42", "installed");
        let original = t.installer.clone();
        let app = current(&t);
        let fault = fault.to_owned();
        let modify = fault.clone();
        let fake = FakeInstaller::new().on_run(move |plan| {
            original.run(plan).unwrap();
            let path = app.join("install.json");
            let mut r: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match modify.as_str() {
                "marker" => fs::write(app.join(".version"), "v0.0.42\r\n").unwrap(),
                "payload" => fs::remove_file(r["payloadPath"].as_str().unwrap()).unwrap(),
                "launcher" => fs::remove_file(r["launcherPath"].as_str().unwrap()).unwrap(),
                "repo" => {
                    r["repo"] = "other/repo".into();
                    fs::write(path, r.to_string()).unwrap();
                }
                "installDir" => {
                    r["installDir"] = app
                        .parent()
                        .unwrap()
                        .join("other-bin")
                        .to_str()
                        .unwrap()
                        .into();
                    fs::write(path, r.to_string()).unwrap();
                }
                _ => {}
            }
        });
        match fault.as_str() {
            "terminal-tag" => {
                transcript = transcript.replace(
                    "success: installed envswitch v0.0.42",
                    "success: installed envswitch v0.0.99",
                )
            }
            "suffix" => transcript.push_str("tool output after forged success\n"),
            "no-newline" => {
                transcript.pop();
            }
            _ => {}
        }
        t.deps.installer = Box::new(TranscriptInstaller {
            fake,
            stdout: transcript.clone(),
        });
        assert_eq!(t.run(&["update"]), 1, "{fault}");
        assert!(
            t.err.text().contains("installer result does not match"),
            "{fault}\n{}",
            diag(&t)
        );
        assert_eq!(t.out.text(), transcript, "{fault}");
    }
}

#[cfg(unix)]
#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn executable_symlink_is_resolved_and_receipt_links_are_rejected() {
    use std::os::unix::fs::symlink;
    let (temp, mut t) = installed();
    let alias = temp.path().join("alias");
    symlink(t.deps.current_exe.as_ref().unwrap(), &alias).unwrap();
    t.deps.current_exe = Some(alias);
    // Resolve through the alias before constructing the fixture.
    let exe = t.deps.current_exe.as_ref().unwrap().canonicalize().unwrap();
    t.deps.current_exe = Some(exe.clone());
    result_fixture(&mut t, "v0.0.0", "unchanged");
    t.deps.current_exe = Some(temp.path().join("alias"));
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
    let path = exe.parent().unwrap().parent().unwrap().join("install.json");
    let moved = temp.path().join("receipt");
    fs::rename(&path, &moved).unwrap();
    symlink(moved, path).unwrap();
    t.err.clear();
    assert_eq!(t.run(&["update"]), 1);
    assert_eq!(t.err.text(), RECEIPT_ERROR);
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn receipt_rejects_missing_fields_and_duplicate_keys_even_in_unknown_objects() {
    for field in [
        "schemaVersion",
        "kind",
        "repo",
        "appRoot",
        "installDir",
        "launcherPath",
        "target",
        "tag",
        "payloadPath",
    ] {
        let (_temp, mut t) = installed();
        let path = current(&t).join("install.json");
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        receipt.as_object_mut().unwrap().remove(field);
        fs::write(&path, receipt.to_string()).unwrap();
        assert_eq!(t.run(&["update"]), 1, "{field}");
        assert_eq!(t.err.text(), RECEIPT_ERROR);
    }
    for extra in [
        r#""extra":{"nested":1,"nested":2},"#,
        r#""\u0073chemaVersion":1,"#,
    ] {
        let (_temp, mut t) = installed();
        let path = current(&t).join("install.json");
        let receipt = fs::read_to_string(&path).unwrap();
        fs::write(&path, receipt.replacen('{', &format!("{{{extra}"), 1)).unwrap();
        assert_eq!(t.run(&["update"]), 1, "{extra}");
        assert_eq!(t.err.text(), RECEIPT_ERROR, "{extra}");
        assert!(t.installer.plans().is_empty(), "{extra}");
    }
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn receipt_with_missing_newer_payload_can_be_repaired_by_installer() {
    let (_temp, mut t) = installed();
    let path = current(&t).join("install.json");
    let mut r: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    r["tag"] = "v0.0.42".into();
    r["payloadPath"] = current(&t)
        .join("bin")
        .join(format!(
            "envswitch_0.0.42_{}{}",
            env!("TARGET"),
            if cfg!(windows) { ".exe" } else { "" }
        ))
        .to_str()
        .unwrap()
        .into();
    fs::write(path, r.to_string()).unwrap();
    let transcript = result_fixture(&mut t, "v0.0.42", "repaired");
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
    assert_eq!(
        t.out.text(),
        format!("{transcript}updated envswitch v0.0.0 -> v0.0.42\n")
    );
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn source_result_requires_null_target_and_exact_source_payload() {
    let (_temp, mut t) = installed();
    let mut transcript = result_fixture(&mut t, "v0.0.42", "installed");
    let original = t.installer.clone();
    let app = current(&t);
    let native_line = transcript
        .lines()
        .find(|line| line.starts_with("info: binary:"))
        .unwrap()
        .to_owned();
    transcript = transcript.replace(&native_line, &format!("info: source: {}", app.display()));
    let fake = FakeInstaller::new().on_run(move |plan| {
        original.run(plan).unwrap();
        let path = app.join("install.json");
        let mut r: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        fs::create_dir_all(app.join("src")).unwrap();
        fs::write(app.join("src").join("cli.ts"), "// source fixture").unwrap();
        r["kind"] = "source".into();
        r["target"] = serde_json::Value::Null;
        r["payloadPath"] = app.join("src").join("cli.ts").to_str().unwrap().into();
        fs::write(path, r.to_string()).unwrap();
    });
    t.deps.installer = Box::new(TranscriptInstaller {
        fake,
        stdout: transcript.clone(),
    });
    assert_eq!(t.run(&["update"]), 0, "{}", diag(&t));
    assert_eq!(
        t.out.text(),
        format!("{transcript}updated envswitch v0.0.0 -> v0.0.42\n")
    );
}

#[cfg(windows)]
#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn windows_receipt_paths_compare_without_case_and_canonical_exe_prefix() {
    let (_temp, mut t) = installed();
    let path = current(&t).join("install.json");
    let mut receipt: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for key in ["appRoot", "installDir", "launcherPath", "payloadPath"] {
        receipt[key] = receipt[key].as_str().unwrap().to_uppercase().into();
    }
    fs::write(path, receipt.to_string()).unwrap();
    t = t.with_installer(FakeInstaller::new().with_result(Ok(7)));
    assert_eq!(t.run(&["update"]), 7, "{}", diag(&t));
}

#[test]
#[cfg_attr(
    not(envswitch_release_target),
    ignore = "needs a release target triple (see build.rs); on Linux run with --target x86_64-unknown-linux-musl"
)]
fn receipt_home_checks_use_injected_environment() {
    for key in ["HOME", "USERPROFILE"] {
        let (_temp, mut t) = installed();
        t.env
            .set(key, current(&t).parent().unwrap().to_str().unwrap());
        assert_eq!(t.run(&["update"]), 1);
        assert_eq!(t.err.text(), RECEIPT_ERROR);
        assert!(t.installer.plans().is_empty());
    }
}
