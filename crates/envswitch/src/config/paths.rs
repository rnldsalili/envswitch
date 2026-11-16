//! Output path safety (contract 02 §2). Owner: C1. Signature frozen by S1.
//!
//! The lexical steps are a port of Node's `path` functions ([`node`]), run in
//! the flavor of the host (`path.win32` on Windows, `path.posix` elsewhere),
//! so every decision the TS oracle makes from strings alone is reproduced
//! exactly. Only the symlink walk touches the filesystem.

pub(crate) mod node;
#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result, fail, io_error};
use crate::jsutil::js_trim;

use node::{CwdFn, DriveCwdFn, posix, win32};

/// TS `resolveOutputPath(root, output)` (02 §2.1, §2.3).
///
/// * Lexical normalization only (never `canonicalize`). A relative `root` is
///   resolved against the process cwd, like Node `path.resolve`.
/// * `User` errors: `target output path cannot be empty`,
///   `... must be relative to the project root: <output>`,
///   `... escapes the project root: <output>` (string `..` prefix test kept,
///   so `..env` is rejected), `... contains a symbolic-link parent: <output>`.
/// * The lstat walk covers every ancestor from the output's parent up to and
///   including the root. `NotFound` continues; any other io error is
///   `crate::error::io_error(err, "lstat", path)` (exit 2, DIVERGENCES d1).
/// * Windows drive-relative paths keep parity (no tightening).
pub fn resolve_output_path(root: &Path, output: &str) -> Result<PathBuf> {
    let flavor = Flavor::NATIVE;
    let lexical = flavor.resolve_lexically(
        &path_to_bytes(root),
        output,
        &process_cwd,
        &process_drive_cwd,
    )?;
    for parent in flavor.parent_chain(&lexical) {
        if parent.contains(&0) {
            // Bun's `lstatSync` argument validation (a TypeError, exit 2).
            let received = serde_json::Value::String(String::from_utf8_lossy(&parent).into_owned());
            return Err(Error::internal(format!(
                "The argument 'path' must be a string, Uint8Array, or URL without null bytes. Received {received}"
            )));
        }
        let parent = bytes_to_path(&parent);
        match fs::symlink_metadata(&parent) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return fail(format!(
                    "target output path contains a symbolic-link parent: {output}"
                ));
            }
            Ok(_) => {}
            Err(err) if is_no_entry(&err) => {}
            Err(err) => return Err(io_error(&err, "lstat", &parent)),
        }
    }
    Ok(bytes_to_path(&lexical.output_abs))
}

/// The result of the string-only steps 1–5 of 02 §2.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lexical {
    /// `path.resolve(root)`.
    pub(crate) root_abs: Vec<u8>,
    /// `path.resolve(rootAbs, path.normalize(output))`.
    pub(crate) output_abs: Vec<u8>,
}

/// Which Node `path` implementation to mirror. Production code only uses
/// [`Flavor::NATIVE`]; both flavors exist on every host so the unit tests can
/// check the win32 rules on Unix and the POSIX rules on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Flavor {
    Posix,
    Win32,
}

impl Flavor {
    /// The flavor Node's default `path` export uses on this host.
    #[cfg(windows)]
    pub(crate) const NATIVE: Flavor = Flavor::Win32;
    /// The flavor Node's default `path` export uses on this host.
    #[cfg(not(windows))]
    pub(crate) const NATIVE: Flavor = Flavor::Posix;

    pub(crate) fn is_absolute(self, path: &[u8]) -> bool {
        match self {
            Flavor::Posix => posix::is_absolute(path),
            Flavor::Win32 => win32::is_absolute(path),
        }
    }

    pub(crate) fn normalize(self, path: &[u8]) -> Vec<u8> {
        match self {
            Flavor::Posix => posix::normalize(path),
            Flavor::Win32 => win32::normalize(path),
        }
    }

    pub(crate) fn resolve(
        self,
        args: &[&[u8]],
        cwd: CwdFn<'_>,
        drive_cwd: DriveCwdFn<'_>,
    ) -> Result<Vec<u8>> {
        match self {
            Flavor::Posix => posix::resolve(args, cwd),
            Flavor::Win32 => win32::resolve(args, cwd, drive_cwd),
        }
    }

    pub(crate) fn relative(
        self,
        from: &[u8],
        to: &[u8],
        cwd: CwdFn<'_>,
        drive_cwd: DriveCwdFn<'_>,
    ) -> Result<Vec<u8>> {
        match self {
            Flavor::Posix => posix::relative(from, to, cwd),
            Flavor::Win32 => win32::relative(from, to, cwd, drive_cwd),
        }
    }

    pub(crate) fn dirname(self, path: &[u8]) -> Vec<u8> {
        match self {
            Flavor::Posix => posix::dirname(path),
            Flavor::Win32 => win32::dirname(path),
        }
    }

    pub(crate) fn basename(self, path: &[u8]) -> Vec<u8> {
        match self {
            Flavor::Posix => posix::basename(path),
            Flavor::Win32 => win32::basename(path),
        }
    }

    pub(crate) fn join(self, args: &[&[u8]]) -> Vec<u8> {
        match self {
            Flavor::Posix => posix::join(args),
            Flavor::Win32 => win32::join(args),
        }
    }

    /// Steps 1–5 of 02 §2.1: emptiness, absolute rejection, lexical
    /// resolution, and the `rel === "" || rel.startsWith("..") ||
    /// isAbsolute(rel)` escape test (which keeps the `..env` false positive).
    pub(crate) fn resolve_lexically(
        self,
        root: &[u8],
        output: &str,
        cwd: CwdFn<'_>,
        drive_cwd: DriveCwdFn<'_>,
    ) -> Result<Lexical> {
        if js_trim(output).is_empty() {
            return fail("target output path cannot be empty");
        }
        let output_bytes = output.as_bytes();
        if self.is_absolute(output_bytes) {
            return fail(format!(
                "target output path must be relative to the project root: {output}"
            ));
        }
        let root_abs = self.resolve(&[root], cwd, drive_cwd)?;
        let normalized = self.normalize(output_bytes);
        let output_abs = self.resolve(&[&root_abs, &normalized], cwd, drive_cwd)?;
        let rel = self.relative(&root_abs, &output_abs, cwd, drive_cwd)?;
        if rel.is_empty() || rel.starts_with(b"..") || self.is_absolute(&rel) {
            return fail(format!(
                "target output path escapes the project root: {output}"
            ));
        }
        Ok(Lexical {
            root_abs,
            output_abs,
        })
    }

    /// The directories the TS `rejectSymlinkedParents` loop lstats, in order:
    /// `dirname(outputAbs)` upward to and including `rootAbs`.
    ///
    /// TS stops only when `current === rootAbs`. On win32 a drive-relative
    /// output whose drive letter differs in case from the root's (`c:x` under
    /// `C:\Proj`) passes the case-insensitive `relative` check but never
    /// reaches a byte-equal root, and the TS loop spins forever at `c:\`.
    /// Rust stops at the case-insensitive root instead (that is the directory
    /// TS meant to reach), and also at any `dirname` fixed point.
    pub(crate) fn parent_chain(self, lexical: &Lexical) -> Vec<Vec<u8>> {
        let mut chain = Vec::new();
        let mut current = self.dirname(&lexical.output_abs);
        loop {
            let at_root = current == lexical.root_abs
                || (self == Flavor::Win32 && current.eq_ignore_ascii_case(&lexical.root_abs));
            let next = if at_root {
                None
            } else {
                Some(self.dirname(&current))
            };
            chain.push(current);
            match next {
                Some(next) if Some(&next) != chain.last() => current = next,
                _ => return chain,
            }
        }
    }
}

/// `true` for the errors Node's `lstatSync(path, { throwIfNoEntry: false })`
/// swallows (libuv `ENOENT`).
fn is_no_entry(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::NotFound {
        return true;
    }
    // libuv maps these Win32 errors to ENOENT as well: BAD_PATHNAME,
    // DIRECTORY, FILE_NOT_FOUND, INVALID_NAME, INVALID_DRIVE,
    // INVALID_REPARSE_DATA, PATH_NOT_FOUND.
    cfg!(windows)
        && matches!(
            err.raw_os_error(),
            Some(2 | 3 | 15 | 123 | 161 | 267 | 4392)
        )
}

/// `process.cwd()` as bytes in the native flavor.
fn process_cwd() -> Result<Vec<u8>> {
    match std::env::current_dir() {
        Ok(cwd) => Ok(path_to_bytes(&cwd)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            Err(Error::internal("ENOENT: no such file or directory, uv_cwd"))
        }
        Err(err) => Err(Error::internal(err.to_string())),
    }
}

/// `process.env["=X:"]`, the per-drive cwd Windows keeps in the environment.
fn process_drive_cwd(device: &[u8]) -> Option<Vec<u8>> {
    if !cfg!(windows) {
        return None;
    }
    let name = format!("={}", String::from_utf8_lossy(device));
    std::env::var_os(name).map(|value| path_to_bytes(Path::new(&value)))
}

/// Path → the string Node would see, as UTF-8 bytes (raw bytes on Unix).
pub(crate) fn path_to_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

/// Inverse of [`path_to_bytes`].
pub(crate) fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Node `path.join(root, name)` in the native flavor (lexical, like TS).
pub(crate) fn native_join(root: &Path, name: &str) -> PathBuf {
    bytes_to_path(&Flavor::NATIVE.join(&[&path_to_bytes(root), name.as_bytes()]))
}

/// Node `path.basename(path)` in the native flavor.
pub(crate) fn native_basename(path: &Path) -> String {
    String::from_utf8_lossy(&Flavor::NATIVE.basename(&path_to_bytes(path))).into_owned()
}
