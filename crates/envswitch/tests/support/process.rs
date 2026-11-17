//! Process fixtures own every writable path; PATH contains only explicit tool links.
#![allow(dead_code)]

use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

const HOMES: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "APPDATA",
    "LOCALAPPDATA",
    "TMPDIR",
    "TEMP",
    "TMP",
];

pub struct Sandbox {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub tools: PathBuf,
    pub project: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        #[cfg(windows)]
        let root = PathBuf::from(root.to_str().unwrap().trim_start_matches("\\\\?\\"));
        let tools = root.join("tools");
        let project = root.join("project");
        for path in [tools.clone(), project.clone()]
            .into_iter()
            .chain(HOMES.iter().map(|h| root.join(h)))
        {
            assert!(path.starts_with(&root));
            fs::create_dir(&path).unwrap();
        }
        Self {
            _temp: temp,
            root,
            tools,
            project,
        }
    }

    pub fn owned(&self, path: &Path) {
        assert!(path.starts_with(&self.root));
        assert!(!path.components().any(|c| c == Component::ParentDir));
        let parent = path.parent().unwrap().canonicalize().unwrap();
        assert!(parent.starts_with(self._temp.path().canonicalize().unwrap()));
        assert!(
            !path
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        );
    }

    pub fn write(&self, path: &Path, bytes: impl AsRef<[u8]>) {
        self.owned(path);
        fs::write(path, bytes).unwrap();
    }

    pub fn link(&self, name: &str, target: &Path) {
        let path = self.tools.join(name);
        self.owned(&path);
        let target = target.canonicalize().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, &path).unwrap();
    }

    pub fn command(&self, exe: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(exe);
        command
            .env_clear()
            .current_dir(&self.project)
            .env("PATH", &self.tools)
            .env("NO_COLOR", "1")
            .env("TERM", "xterm");
        for home in HOMES {
            command.env(home, self.root.join(home));
        }
        #[cfg(windows)]
        {
            if let Some(system_root) = std::env::var_os("SystemRoot") {
                command
                    .env("SystemRoot", &system_root)
                    .env("WINDIR", system_root);
            }
            command.env("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        }
        command
    }

    pub fn fake_op(&self, fixture: &serde_json::Value) {
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            assert!(
                !Path::new(&system_root).join("System32/op.exe").exists(),
                "System32/op.exe would take precedence over the sandbox fake"
            );
        }
        self.link(
            if cfg!(windows) { "op.exe" } else { "op" },
            &fake_op_binary(),
        );
        self.write(&self.root.join("op.json"), fixture.to_string());
        self.write(&self.root.join("trace.jsonl"), []);
        let output = self
            .op_command(self.tools.join(if cfg!(windows) { "op.exe" } else { "op" }))
            .arg("__envswitch_fake_handshake")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"envswitch-fake-op-v1\n");
        assert!(output.stderr.is_empty());
    }

    pub fn op_command(&self, exe: impl AsRef<OsStr>) -> Command {
        let mut command = self.command(exe);
        command
            .env("ENVSWITCH_FAKE_OP_FIXTURE", self.root.join("op.json"))
            .env("ENVSWITCH_FAKE_OP_TRACE", self.root.join("trace.jsonl"));
        command
    }

    pub fn cli(&self) -> Command {
        // Keep the CLI away from any sibling op.exe (Windows searches there first).
        let dir = self.root.join("cli");
        if !dir.exists() {
            self.owned(&dir);
            fs::create_dir(&dir).unwrap();
            let exe = dir.join(if cfg!(windows) {
                "envswitch.exe"
            } else {
                "envswitch"
            });
            self.owned(&exe);
            fs::copy(env!("CARGO_BIN_EXE_envswitch"), exe).unwrap();
        }
        self.op_command(dir.join(if cfg!(windows) {
            "envswitch.exe"
        } else {
            "envswitch"
        }))
    }
}

pub fn resolve_tool(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(name))
        .find_map(|p| p.is_file().then(|| p.canonicalize().unwrap()))
        .unwrap_or_else(|| panic!("required test tool missing: {name}"))
}

/// The native fake `op` next to the CLI under test, building it (once per test
/// binary) when the profile directory does not have it yet.
pub fn fake_op_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let profile = Path::new(env!("CARGO_BIN_EXE_envswitch")).parent().unwrap();
            let fake = profile.join(if cfg!(windows) { "op.exe" } else { "op" });
            if !fake.is_file() {
                // Cargo does not expose CARGO_BIN_EXE_op across workspace packages.
                let sandbox = Sandbox::new();
                let cargo = Path::new(env!("CARGO"));
                let toolchain = cargo.parent().unwrap();
                let rustc = toolchain.join(if cfg!(windows) { "rustc.exe" } else { "rustc" });
                for name in if cfg!(windows) {
                    &[][..]
                } else {
                    &["cc", "ar"][..]
                } {
                    sandbox.link(name, &resolve_tool(name));
                }
                let mut command = sandbox.command(cargo);
                command
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .args([
                        "build",
                        "--locked",
                        "-p",
                        "envswitch-fake-op",
                        "--target-dir",
                    ])
                    .arg(profile.parent().unwrap())
                    .env("RUSTC", rustc);
                for name in [
                    "CARGO_HOME",
                    "RUSTUP_HOME",
                    "LIB",
                    "LIBPATH",
                    "INCLUDE",
                    "SDKROOT",
                    "DEVELOPER_DIR",
                ] {
                    if let Some(value) = std::env::var_os(name) {
                        command.env(name, value);
                    }
                }
                if std::env::var_os("CARGO_HOME").is_none() {
                    command.env(
                        "CARGO_HOME",
                        Path::new(&std::env::var_os("HOME").unwrap()).join(".cargo"),
                    );
                }
                if profile.file_name().unwrap() == "release" {
                    command.arg("--release");
                }
                let output = command.output().unwrap();
                assert!(
                    output.status.success(),
                    "building fake-op failed: {output:?}"
                );
            }
            assert!(fake.is_file(), "missing fake-op at {}", fake.display());
            fake
        })
        .clone()
}
