use super::json::J;
use serde_json::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
#[derive(Clone)]
pub struct Bridge {
    pub repo: PathBuf,
    pub bun: PathBuf,
}
impl Bridge {
    pub fn new(repo: &Path) -> Self {
        let requested = std::env::var_os("ENVSWITCH_PARITY_BUN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "bun.exe" } else { "bun" }));
        let bun = if requested.is_absolute() {
            requested
        } else {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|p| p.join(&requested))
                .find(|p| p.is_file())
                .unwrap_or(requested)
        };
        Self {
            repo: repo.into(),
            bun,
        }
    }
    pub fn run(&self, args: &[String], input: Option<&str>) -> Result<Value, String> {
        let mut cmd = Command::new(&self.bun);
        cmd.arg(self.repo.join("parity/adapters/rust/bridge.ts"))
            .args(args)
            .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Bun bridge unavailable: {e}"))?;
        if let Some(text) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "bridge failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        if output.stdout.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
        }
    }
    pub fn actions(&self, bundle: &Path, actions: &J) {
        self.run(
            &["actions".into(), bundle.display().to_string()],
            Some(&actions.json()),
        )
        .expect("mutation actions");
    }
    pub fn decode_base64(&self, text: &str) -> Vec<u8> {
        self.run(&["decode-base64".into(), text.into()], None)
            .expect("base64 transport")
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect()
    }
    pub fn cleanup(&self, bundle: &Path) {
        self.run(&["cleanup".into(), bundle.display().to_string()], None)
            .expect("sandbox cleanup");
    }
}
pub struct CwdGuard(PathBuf);
impl CwdGuard {
    pub fn set(cwd: &Path) -> Self {
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(cwd).unwrap();
        Self(old)
    }
}
impl Drop for CwdGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).expect("restore cwd")
    }
}
#[cfg(target_os = "macos")]
type Mode = u16;
#[cfg(all(unix, not(target_os = "macos")))]
type Mode = u32;
#[cfg(unix)]
unsafe extern "C" {
    fn umask(mode: Mode) -> Mode;
}
pub struct UmaskGuard {
    #[cfg(unix)]
    old: Mode,
}
impl UmaskGuard {
    pub fn set(value: u32) -> Self {
        #[cfg(unix)]
        {
            Self {
                old: unsafe { umask(value as Mode) },
            }
        }
        #[cfg(not(unix))]
        {
            let _ = value;
            Self {}
        }
    }
}
impl Drop for UmaskGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            umask(self.old);
        }
    }
}
