//! Temporary project roots (`tempfile`), never under the real `HOME`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// A project root inside a fresh temp directory, deleted on drop.
/// The root is `<tmp>/<name>` so `basename(root)` is predictable
/// (`envswitch-project` by default, as in the TS guided tests).
pub struct TempProject {
    _tmp: TempDir,
    root: PathBuf,
}

impl Default for TempProject {
    fn default() -> Self {
        Self::new()
    }
}

impl TempProject {
    pub fn new() -> Self {
        Self::named("envswitch-project")
    }

    pub fn named(name: &str) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("envswitch-test-")
            .tempdir()
            .expect("create temp dir");
        let root = tmp.path().join(name);
        fs::create_dir_all(&root).expect("create project root");
        TempProject { _tmp: tmp, root }
    }

    /// The project root (not canonicalized).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The temp directory that contains the root (for "outside" paths).
    pub fn outside(&self) -> &Path {
        self._tmp.path()
    }

    /// `root.join(rel)`.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// `mkdir -p root/rel`.
    pub fn dir(&self, rel: &str) -> &Self {
        fs::create_dir_all(self.path(rel)).expect("create dir");
        self
    }

    /// Writes `root/rel` (parents created).
    pub fn file(&self, rel: &str, contents: &str) -> &Self {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(path, contents).expect("write file");
        self
    }

    /// Writes `root/package.json` with `{"name": <name>}`.
    pub fn package_json(&self, name: &str) -> &Self {
        let json = serde_json::json!({ "name": name });
        self.file("package.json", &format!("{json}\n"))
    }

    /// Contents of `root/rel`, or `None` if it cannot be read.
    pub fn read(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.path(rel)).ok()
    }

    /// Sorted entry names of `root/rel` (empty if missing).
    pub fn entries(&self, rel: &str) -> Vec<String> {
        let mut names: Vec<String> = match fs::read_dir(self.path(rel)) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    /// Makes `root/link_rel` a directory link to `target`: a symlink on Unix,
    /// a junction on Windows (like the TS tests).
    pub fn link_dir(&self, link_rel: &str, target: &Path) -> io::Result<()> {
        let link = self.path(link_rel);
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent)?;
        }
        link_dir(target, &link)
    }
}

/// Creates a directory link at `link` pointing to `target`.
#[cfg(unix)]
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Creates a directory link at `link` pointing to `target`.
#[cfg(windows)]
pub fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    junction::create(target, link)
}
