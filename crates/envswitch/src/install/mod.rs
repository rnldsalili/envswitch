//! Native update handoff to the embedded protocol-1 installers.

mod console;
pub mod embed;
pub(crate) mod process;
pub mod receipt;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

use crate::error::{Error, Result};
use crate::ports::{InstallPlan, InstallerRunner};

/// Restrict the handoff directory before writing executable installer bytes.
#[cfg(windows)]
const SECURE_TEMP_DIRECTORY: &str = "$ErrorActionPreference='Stop'; $p=$env:ENVSWITCH_INSTALLER_TEMP; $a=New-Object System.Security.AccessControl.DirectorySecurity; $a.SetAccessRuleProtection($true,$false); $u=[System.Security.Principal.WindowsIdentity]::GetCurrent().User; foreach($s in @($u,(New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-18'),(New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-32-544'))) { $r=New-Object System.Security.AccessControl.FileSystemAccessRule($s,'FullControl','ContainerInherit,ObjectInherit','None','Allow'); $a.AddAccessRule($r) }; Set-Acl -LiteralPath $p -AclObject $a";

#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddedInstaller;

impl InstallerRunner for EmbeddedInstaller {
    fn run(&self, plan: &InstallPlan) -> Result<i32> {
        self.run_with_output(plan, &mut std::io::stdout())
            .map(|(code, _)| code)
    }

    fn run_with_output(&self, plan: &InstallPlan, out: &mut dyn Write) -> Result<(i32, String)> {
        self.run_with_writers(plan, out, &mut std::io::stderr())
    }

    fn run_with_writers(
        &self,
        plan: &InstallPlan,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> Result<(i32, String)> {
        run_embedded(plan, out, err)
    }
}

struct PrivateDir(PathBuf);
impl PrivateDir {
    fn create(plan: &InstallPlan) -> Result<Self> {
        let path = receipt::canonical_path(&std::env::temp_dir())
            .map_err(launch_error)?
            .join(format!("envswitch-update-{}", uuid::Uuid::new_v4()));
        if path
            .ancestors()
            .any(|p| receipt::same_path(p, &plan.app_root.join("current")))
        {
            return Err(Error::user(
                "update failed: could not run installer: temporary directory is inside current",
            ));
        }
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let mut builder = builder;
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(launch_error)?;
        let dir = Self(path);
        #[cfg(windows)]
        {
            // Set an owner-only ACL before writing any executable script bytes.
            let status = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    SECURE_TEMP_DIRECTORY,
                ])
                .env("ENVSWITCH_INSTALLER_TEMP", &dir.0)
                .status()
                .map_err(spawn_error)?;
            if !status.success() {
                return Err(Error::user(
                    "update failed: could not run installer: could not secure temporary directory",
                ));
            }
        }
        Ok(dir)
    }
}
impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn launch_error(e: std::io::Error) -> Error {
    Error::user(format!("update failed: could not run installer: {e}"))
}

fn spawn_error(e: std::io::Error) -> Error {
    Error::internal(format!("update failed: could not run installer: {e}"))
}

fn status_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
    }
    #[cfg(not(unix))]
    {
        status.code().unwrap_or(1)
    }
}

fn run_embedded(
    plan: &InstallPlan,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(i32, String)> {
    let install_dir = plan
        .install_dir
        .as_ref()
        .ok_or_else(|| Error::user(receipt::RECEIPT_GUIDANCE))?;
    if plan
        .version
        .as_deref()
        .is_some_and(|v| !receipt::canonical_tag(v))
        || (!plan.current_version.is_empty() && !receipt::canonical_tag(&plan.current_version))
    {
        return Err(Error::user(
            "update failed: could not run installer: invalid version tag",
        ));
    }
    let handoff = process::Handoff::new().map_err(launch_error)?;
    let temp = PrivateDir::create(plan)?;
    let (name, script) = embed::installer_script();
    let script_path = temp.0.join(name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Close the script handle before PowerShell/installer replacement work.
    {
        options
            .open(&script_path)
            .and_then(|mut f| f.write_all(script.as_bytes()))
            .map_err(launch_error)?;
    }
    let mut command = Command::new(if cfg!(windows) { "powershell" } else { "bash" });
    if cfg!(windows) {
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script_path)
            .arg("-AppRoot")
            .arg(&plan.app_root)
            .arg("-InstallDir")
            .arg(install_dir)
            .arg("-Repo")
            .arg(&plan.repo);
        if let Some(pin) = &plan.version {
            command.arg("-Version").arg(pin);
        }
        if !plan.current_version.is_empty() {
            command.arg("-CurrentVersion").arg(&plan.current_version);
        }
        if plan.verify_attestations {
            command.arg("-VerifyAttestations");
        }
    } else {
        command
            .arg(&script_path)
            .env("REPO", &plan.repo)
            .env("APP_ROOT", &plan.app_root)
            .env("INSTALL_DIR", install_dir);
        if let Some(pin) = &plan.version {
            command.env("ENVSWITCH_VERSION", pin);
        }
        if !plan.current_version.is_empty() {
            command.env("ENVSWITCH_CURRENT_VERSION", &plan.current_version);
        }
        if plan.verify_attestations {
            command.env("ENVSWITCH_VERIFY_ATTESTATIONS", "1");
        }
    }
    // Remove stale caller pins even on Windows, which uses explicit parameters.
    if plan.version.is_none() {
        command.env_remove("ENVSWITCH_VERSION");
    }
    if plan.current_version.is_empty() {
        command.env_remove("ENVSWITCH_CURRENT_VERSION");
    }
    if !plan.verify_attestations {
        command.env_remove("ENVSWITCH_VERIFY_ATTESTATIONS");
    }
    handoff.prepare(&mut command);
    #[cfg(windows)]
    let encoding = console::Encoding::new();
    let mut child = command
        .current_dir(&temp.0)
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    handoff.started(child.id());
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let mut write_error = None;
    let read_result = loop {
        match stdout.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                bytes.extend_from_slice(&buffer[..n]);
                if write_error.is_none() {
                    #[cfg(not(windows))]
                    {
                        write_error = out.write_all(&buffer[..n]).and_then(|()| out.flush()).err();
                    }
                    #[cfg(windows)]
                    {
                        let _ = &out;
                        write_error = console::write(&buffer[..n]).err();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e),
        }
    };
    drop(stdout);
    let status = child.wait().map_err(spawn_error)?;
    handoff.finished();
    let cleanup = fs::remove_dir_all(&temp.0);
    let code = status_code(status);
    if code == 0 {
        read_result.map_err(launch_error)?;
        if let Some(e) = write_error {
            let _ = writeln!(
                err,
                "envswitch: warning: could not forward installer stdout: {e}"
            );
        }
        if let Err(e) = cleanup {
            let _ = writeln!(
                err,
                "envswitch: warning: could not remove installer temporary directory {}: {e}",
                temp.0.display()
            );
        }
    }
    #[cfg(windows)]
    let output = encoding.decode(&bytes);
    #[cfg(not(windows))]
    let output = String::from_utf8_lossy(&bytes).into_owned();
    Ok((code, output))
}
