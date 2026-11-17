//! Native update handoff to the embedded protocol-1 installers.

mod console;
pub mod embed;
pub(crate) mod process;
pub mod receipt;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::error::{Error, Result};
use crate::ports::{InstallPlan, InstallerRunner};

/// Restrict the handoff directory before writing executable installer bytes.
#[cfg_attr(not(windows), allow(dead_code))]
const SECURE_TEMP_DIRECTORY: &str = "$ErrorActionPreference='Stop'; $p=$env:ENVSWITCH_INSTALLER_TEMP; $a=New-Object System.Security.AccessControl.DirectorySecurity; $a.SetAccessRuleProtection($true,$false); $u=[System.Security.Principal.WindowsIdentity]::GetCurrent().User; foreach($s in @($u,(New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-18'),(New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-32-544'))) { $r=New-Object System.Security.AccessControl.FileSystemAccessRule($s,'FullControl','ContainerInherit,ObjectInherit','None','Allow'); $a.AddAccessRule($r) }; Set-Acl -LiteralPath $p -AclObject $a";

/// Windows PowerShell 5.1 (`powershell.exe`) without the caller's `PSModulePath`.
///
/// When envswitch is started from PowerShell 7 (`pwsh`), the inherited
/// `PSModulePath` lists pwsh's own module directories first. Windows
/// PowerShell 5.1 then fails to autoload built-in modules
/// (Microsoft.PowerShell.Security, Utility, Archive), so cmdlets such as
/// `Set-Acl`, `Get-FileHash` and `Expand-Archive` fail. Without the variable,
/// 5.1 computes its default module path. Every 5.1 child must come from here.
fn windows_powershell() -> Command {
    let mut command = Command::new("powershell");
    command.env_remove("PSModulePath");
    command
}

/// The owner-only ACL command for the handoff directory (Windows only).
#[cfg_attr(not(windows), allow(dead_code))]
fn secure_temp_command(dir: &Path) -> Command {
    let mut command = windows_powershell();
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SECURE_TEMP_DIRECTORY,
        ])
        .env("ENVSWITCH_INSTALLER_TEMP", dir);
    command
}

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
            let status = secure_temp_command(&dir.0).status().map_err(spawn_error)?;
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

/// The installer invocation: Windows PowerShell 5.1 `-File` with explicit
/// parameters on Windows, `bash` with environment parameters elsewhere.
fn installer_command(
    plan: &InstallPlan,
    install_dir: &Path,
    script_path: &Path,
    windows: bool,
) -> Command {
    let mut command = if windows {
        windows_powershell()
    } else {
        Command::new("bash")
    };
    if windows {
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script_path)
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
            .arg(script_path)
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
    command
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
    let mut command = installer_command(plan, install_dir, &script_path, cfg!(windows));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn removes_psmodulepath(command: &Command) -> bool {
        command
            .get_envs()
            .any(|(key, value)| key.eq_ignore_ascii_case("PSModulePath") && value.is_none())
    }

    fn plan() -> InstallPlan {
        InstallPlan {
            repo: "demo/suite".into(),
            app_root: PathBuf::from("app"),
            install_dir: Some(PathBuf::from("bin")),
            version: Some("v0.0.42".into()),
            current_version: "v0.0.0".into(),
            verify_attestations: false,
        }
    }

    #[test]
    fn every_windows_powershell_child_drops_inherited_psmodulepath() {
        let plan = plan();
        let commands = [
            secure_temp_command(Path::new("temp")),
            installer_command(&plan, Path::new("bin"), Path::new("install.ps1"), true),
        ];
        for command in &commands {
            assert_eq!(command.get_program(), "powershell");
            assert!(removes_psmodulepath(command), "{command:?}");
        }
    }

    #[test]
    fn unix_installer_keeps_the_callers_environment() {
        let command = installer_command(&plan(), Path::new("bin"), Path::new("install.sh"), false);
        assert_eq!(command.get_program(), "bash");
        assert!(
            !command
                .get_envs()
                .any(|(key, _)| key.eq_ignore_ascii_case("PSModulePath"))
        );
    }

    #[cfg(windows)]
    const PSMODULEPATH_PROBE: &str = "ENVSWITCH_TEST_PSMODULEPATH_PROBE";

    /// Runs the probe below in a child test process whose own environment
    /// carries a pwsh 7 style `PSModulePath`, as when envswitch is started
    /// from pwsh, and checks real Windows PowerShell 5.1 children.
    #[cfg(windows)]
    #[test]
    fn windows_powershell_ignores_psmodulepath_inherited_from_pwsh() {
        let sentinel = tempfile::tempdir().unwrap();
        let mut inherited = std::ffi::OsString::new();
        let pwsh_modules = Path::new(r"C:\Program Files\PowerShell\7\Modules");
        if pwsh_modules.is_dir() {
            inherited.push(pwsh_modules);
            inherited.push(";");
        }
        inherited.push(sentinel.path());
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "install::tests::psmodulepath_probe",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("PSModulePath", &inherited)
            .env(PSMODULEPATH_PROBE, sentinel.path())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(
            stdout.contains("1 passed"),
            "probe did not run:\n{stdout}\n{stderr}"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "helper process for windows_powershell_ignores_psmodulepath_inherited_from_pwsh"]
    fn psmodulepath_probe() {
        let Some(sentinel) = std::env::var_os(PSMODULEPATH_PROBE) else {
            return;
        };
        let inherited = std::env::var_os("PSModulePath").unwrap_or_default();
        assert!(
            inherited
                .to_string_lossy()
                .contains(&*sentinel.to_string_lossy()),
            "the probe must inherit the pwsh-style PSModulePath"
        );
        // Set-Acl lives in Microsoft.PowerShell.Security.
        let dir = tempfile::tempdir().unwrap();
        let status = secure_temp_command(dir.path()).status().unwrap();
        assert!(status.success(), "securing the handoff directory failed");
        let file = dir.path().join("probe.txt");
        fs::write(&file, b"probe").unwrap();
        let output = windows_powershell()
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "$ErrorActionPreference='Stop'; \
                 if ($env:PSModulePath.IndexOf($env:ENVSWITCH_PROBE_SENTINEL, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) { throw 'inherited PSModulePath reached Windows PowerShell 5.1' }; \
                 Import-Module Microsoft.PowerShell.Security, Microsoft.PowerShell.Utility, Microsoft.PowerShell.Archive; \
                 $null = Get-FileHash -LiteralPath $env:ENVSWITCH_PROBE_FILE -Algorithm SHA256",
            ])
            .env("ENVSWITCH_PROBE_SENTINEL", &sentinel)
            .env("ENVSWITCH_PROBE_FILE", &file)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
