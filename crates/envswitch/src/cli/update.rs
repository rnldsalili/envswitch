//! Native update: locate a managed install, hand off, then verify committed results.

use std::fs;

use crate::error::{Error, Result};
use crate::install::receipt::{Receipt, canonical_tag, locate, no_links, read_receipt, same_path};
use crate::model::{ProgressEvent, ProgressStatus};
use crate::ports::{Deps, InstallPlan};

use super::args::ParsedArgs;
use super::output::{log_progress, write_line};

const POSTCONDITION: &str = "update failed: installer result does not match the installed receipt and version marker; reinstall envswitch with the install script from the README";

pub(crate) fn run_update(_args: &ParsedArgs, deps: &mut Deps) -> Result<()> {
    let before = locate(
        deps.current_exe.as_deref(),
        deps.app_dir.as_deref(),
        &deps.env,
    )?;
    let old = format!("v{}", env!("CARGO_PKG_VERSION"));
    let plan = InstallPlan {
        repo: before.repo.clone(),
        app_root: before.app_root.clone(),
        install_dir: Some(before.install_dir.clone()),
        version: None,
        current_version: old.clone(),
        verify_attestations: false,
    };
    log_progress(
        deps,
        ProgressEvent {
            status: ProgressStatus::Info,
            message: format!("updating envswitch {old} -> latest"),
        },
    );
    let (code, output) =
        deps.installer
            .run_with_writers(&plan, deps.out.as_mut(), deps.err.as_mut())?;
    if code != 0 {
        return Err(Error::status(
            code,
            format!("update failed: installer exited with code {code}"),
        ));
    }
    let after =
        read_receipt(&before.app_root, &deps.env).ok_or_else(|| Error::user(POSTCONDITION))?;
    let marker_path = after.app_root.join("current").join(".version");
    let marker = fs::read(&marker_path).ok();
    if after.repo != before.repo
        || !same_path(&after.install_dir, &before.install_dir)
        || !same_path(&after.launcher_path, &before.launcher_path)
        || marker.as_deref() != Some(format!("{}\n", after.tag).as_bytes())
        || !no_links(&marker_path)
        || !after.payload_path.is_file()
        || !after.launcher_path.is_file()
    {
        return Err(Error::user(POSTCONDITION));
    }
    let outcome = disposition(&output, &after, deps.err.as_mut())
        .ok_or_else(|| Error::user(POSTCONDITION))?;
    let line = if after.tag != old || outcome == "installed" {
        format!("updated envswitch {old} -> {}", after.tag)
    } else if outcome == "repaired" {
        format!("repaired envswitch {}", after.tag)
    } else {
        format!("envswitch is already up to date ({})", after.tag)
    };
    write_line(deps, line)
}

fn disposition(
    output: &str,
    receipt: &Receipt,
    err: &mut dyn std::io::Write,
) -> Option<&'static str> {
    if !output.ends_with('\n') {
        return None;
    }
    let lines: Vec<_> = output.lines().collect();
    let index = lines.iter().rposition(|s| s.starts_with("success: "))?;
    let line = lines[index];
    let tag = &receipt.tag;
    if !canonical_tag(tag) {
        return None;
    }
    let outcome = if line == format!("success: envswitch is already up to date ({tag})") {
        if index + 1 != lines.len() {
            return None;
        }
        return Some("unchanged");
    } else if line == format!("success: installed envswitch {tag}") {
        "installed"
    } else if line == format!("success: repaired envswitch {tag}") {
        "repaired"
    } else {
        return None;
    };
    let location = if receipt.kind == "native" {
        format!("info: binary: {}", receipt.payload_path.display())
    } else {
        format!(
            "info: source: {}",
            receipt.app_root.join("current").display()
        )
    };
    let launcher = format!("info: launcher: {}", receipt.launcher_path.display());
    let suffix = &lines[index + 1..];
    let locations_match = if cfg!(windows) {
        // Code-page output may replace unrepresentable characters. The receipt
        // and filesystem above are authoritative; require the defined suffix.
        suffix.first().is_some_and(|s| {
            s.starts_with(if receipt.kind == "native" {
                "info: binary: "
            } else {
                "info: source: "
            })
        }) && suffix
            .get(1)
            .is_some_and(|s| s.starts_with("info: launcher: "))
    } else {
        suffix.first().copied() == Some(location.as_str())
            && suffix.get(1).copied() == Some(launcher.as_str())
    };
    if !locations_match {
        return None;
    }
    if cfg!(windows) && (suffix[0] != location || suffix[1] != launcher) {
        let _ = writeln!(
            err,
            "envswitch: warning: installer locations could not be matched after console decoding; validated the installed receipt instead"
        );
    }
    let guidance = &suffix[2..];
    if cfg!(windows) {
        return (guidance.len() == 3
            && guidance[0] == "info: restart your terminal, or run this command now:"
            && guidance[1].starts_with("info: $env:Path = \"")
            && guidance[1].ends_with(";$env:Path\"")
            && guidance[2] == "info: then run 'envswitch --help' to verify the installation")
            .then_some(outcome);
    }
    if guidance == ["info: run 'envswitch --help' to verify the installation"] {
        return Some(outcome);
    }
    let dir = receipt.install_dir.to_str()?;
    let escaped = dir
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    let command = format!("info: export PATH=\"{escaped}:$PATH\"");
    (guidance
        == [
            "info: restart your terminal, or run this command now:",
            command.as_str(),
            "info: then run 'envswitch --help' to verify the installation",
        ])
    .then_some(outcome)
}
