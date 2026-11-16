//! Error taxonomy and exit-code mapping (contract 01 §4.5, §7; 02 §0).
//!
//! | Variant    | TS origin                                              | Exit |
//! |------------|--------------------------------------------------------|------|
//! | `User`     | `EnvswitchError` thrown by `fail()`                    | 1    |
//! | `Internal` | any other thrown value: `op` failures, fs errors, ...  | 2    |
//! | `Cancelled`| `ExitPromptError` (Ctrl-C / Esc in a prompt, D3)       | 130  |
//!
//! `Display` is the bare message. The CLI prints `envswitch: {msg}` to stderr
//! for `User` and `Internal`, and prints nothing for `Cancelled`.
//! No stack traces, no `cause` chains, never `Debug` output.

use std::io;
use std::path::Path;

/// The single error type used across the crate. Cloneable so test fakes can
/// replay a scripted error more than once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A user-facing validation failure (`fail(...)` in TS). Exit code 1.
    #[error("{0}")]
    User(String),
    /// Any other failure: `op` subprocess errors, filesystem errors, invalid
    /// env keys in `stringify_env`, prompt I/O errors. Exit code 2.
    #[error("{0}")]
    Internal(String),
    /// The user cancelled a prompt. Exit code 130; the CLI prints nothing.
    /// The `Display` text exists only for debugging and is never printed.
    #[error("prompt cancelled")]
    Cancelled,
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// `Error::User(msg)`.
    pub fn user(message: impl Into<String>) -> Self {
        Error::User(message.into())
    }

    /// `Error::Internal(msg)`.
    pub fn internal(message: impl Into<String>) -> Self {
        Error::Internal(message.into())
    }

    /// Process exit code for this error: 1, 2, or 130.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::User(_) => 1,
            Error::Internal(_) => 2,
            Error::Cancelled => 130,
        }
    }
}

/// Rust spelling of TS `fail(message)`: always returns `Err(Error::User)`.
///
/// ```
/// # use envswitch::error::{fail, Result};
/// fn check(ok: bool) -> Result<()> {
///     if !ok {
///         return fail("config.targets must be a non-empty array");
///     }
///     Ok(())
/// }
/// assert_eq!(check(false).unwrap_err().exit_code(), 1);
/// ```
pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::User(message.into()))
}

/// Node-style io error, as an `Error::Internal` (exit 2), per DIVERGENCES d1.
///
/// For ENOENT, ENOTDIR, EACCES, EPERM, EEXIST and EISDIR the message is
/// `CODE: description, syscall 'path'`, e.g.
/// `ENOTDIR: not a directory, lstat '/root/afile/sub'`.
/// Any other error renders as the plain `std::io::Error` text.
///
/// `syscall` is the Node syscall name the TS code would have hit
/// (`lstat`, `stat`, `mkdir`, `open`, `chmod`, `rename`, `unlink`, ...).
pub fn io_error(err: &io::Error, syscall: &str, path: &Path) -> Error {
    Error::Internal(node_io_message(err, syscall, path, None))
}

/// Like [`io_error`] for two-path syscalls (`rename`, `link`, `copyfile`):
/// Node renders `CODE: description, rename 'src' -> 'dest'`.
pub fn io_error_dest(err: &io::Error, syscall: &str, path: &Path, dest: &Path) -> Error {
    Error::Internal(node_io_message(err, syscall, path, Some(dest)))
}

/// Builds the message used by [`io_error`] / [`io_error_dest`].
pub fn node_io_message(err: &io::Error, syscall: &str, path: &Path, dest: Option<&Path>) -> String {
    match node_code(err) {
        Some((code, description)) => match dest {
            Some(dest) => format!(
                "{code}: {description}, {syscall} '{}' -> '{}'",
                path.display(),
                dest.display()
            ),
            None => format!("{code}: {description}, {syscall} '{}'", path.display()),
        },
        None => err.to_string(),
    }
}

/// Maps an io error to the libuv code and description Node prints, for the
/// six codes covered by DIVERGENCES d1. `None` for everything else.
fn node_code(err: &io::Error) -> Option<(&'static str, &'static str)> {
    const ENOENT: (&str, &str) = ("ENOENT", "no such file or directory");
    const ENOTDIR: (&str, &str) = ("ENOTDIR", "not a directory");
    const EACCES: (&str, &str) = ("EACCES", "permission denied");
    const EPERM: (&str, &str) = ("EPERM", "operation not permitted");
    const EEXIST: (&str, &str) = ("EEXIST", "file already exists");
    const EISDIR: (&str, &str) = ("EISDIR", "illegal operation on a directory");

    if let Some(raw) = err.raw_os_error() {
        // errno values are identical on Linux and macOS for these codes.
        #[cfg(unix)]
        let mapped = match raw {
            1 => Some(EPERM),
            2 => Some(ENOENT),
            13 => Some(EACCES),
            17 => Some(EEXIST),
            20 => Some(ENOTDIR),
            21 => Some(EISDIR),
            _ => None,
        };
        // Win32 error codes, translated the way libuv does.
        #[cfg(windows)]
        let mapped = match raw {
            2 | 3 => Some(ENOENT),    // FILE_NOT_FOUND, PATH_NOT_FOUND
            5 => Some(EPERM),         // ACCESS_DENIED
            80 | 183 => Some(EEXIST), // FILE_EXISTS, ALREADY_EXISTS
            267 => Some(ENOTDIR),     // DIRECTORY
            _ => None,
        };
        #[cfg(not(any(unix, windows)))]
        let mapped: Option<(&str, &str)> = {
            let _ = raw;
            None
        };
        return mapped;
    }

    match err.kind() {
        io::ErrorKind::NotFound => Some(ENOENT),
        io::ErrorKind::NotADirectory => Some(ENOTDIR),
        io::ErrorKind::AlreadyExists => Some(EEXIST),
        io::ErrorKind::IsADirectory => Some(EISDIR),
        io::ErrorKind::PermissionDenied => Some(EACCES),
        _ => None,
    }
}
