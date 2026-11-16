//! Protocol-1 install receipts. Receipt bytes never authorize guessed locations.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

pub const RECEIPT_GUIDANCE: &str = "cannot locate a valid envswitch install receipt for this binary; reinstall envswitch with the install script from the README";
pub const SOURCE_GUIDANCE: &str = "envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub kind: String,
    pub repo: String,
    pub app_root: PathBuf,
    pub install_dir: PathBuf,
    pub launcher_path: PathBuf,
    pub target: Option<String>,
    pub tag: String,
    pub payload_path: PathBuf,
}

pub(crate) fn canonical_tag(tag: &str) -> bool {
    let Some(n) = tag.strip_prefix("v0.0.") else {
        return false;
    };
    n == "0" || (!n.starts_with('0') && !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

fn valid_repo(repo: &str) -> bool {
    let parts: Vec<_> = repo.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        })
}

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CompareStringOrdinal(
                a: *const u16,
                alen: i32,
                b: *const u16,
                blen: i32,
                ignore_case: i32,
            ) -> i32;
        }
        let a: Vec<_> = a.as_os_str().encode_wide().collect();
        let b: Vec<_> = b.as_os_str().encode_wide().collect();
        let (Ok(alen), Ok(blen)) = (i32::try_from(a.len()), i32::try_from(b.len())) else {
            return false;
        };
        // SAFETY: both buffers remain live for their explicit UTF-16 lengths.
        unsafe { CompareStringOrdinal(a.as_ptr(), alen, b.as_ptr(), blen, 1) == 2 }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

pub(crate) fn canonical_path(path: &Path) -> std::io::Result<PathBuf> {
    let resolved = path.canonicalize()?;
    #[cfg(windows)]
    if let Some(s) = resolved.to_str().and_then(|s| s.strip_prefix("\\\\?\\")) {
        return Ok(PathBuf::from(s));
    }
    Ok(resolved)
}

pub(crate) fn no_links(path: &Path) -> bool {
    path.ancestors().all(|p| match fs::symlink_metadata(p) {
        Ok(m) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                m.file_attributes() & 0x400 == 0
            }
            #[cfg(not(windows))]
            {
                !m.file_type().is_symlink()
            }
        }
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    })
}

fn valid_path(path: &Path) -> bool {
    let Some(s) = path.to_str() else { return false };
    if !path.is_absolute()
        || path.parent().is_none()
        || s.chars().any(char::is_control)
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        || !no_links(path)
    {
        return false;
    }
    #[cfg(unix)]
    {
        !s.contains(':')
            && !s.contains("..")
            && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
            && !s.contains('/')
            && !s[2..].contains(':')
            && !s.contains('"')
            && s.split('\\')
                .skip(1)
                .all(|part| !part.is_empty() && !part.ends_with(['.', ' ']))
    }
}

/// Parse an object while rejecting duplicate decoded keys in every nested object.
/// `Value` alone would silently keep the last duplicate, including escaped keys.
fn parse_receipt_without_duplicate_keys(bytes: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    value.as_object()?;
    let mut i = 0;
    let mut objects: Vec<HashSet<String>> = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if bytes[i + 1..].iter().find(|c| !c.is_ascii_whitespace()) == Some(&b':') {
                    let name: String = serde_json::from_slice(&bytes[start..=i]).ok()?;
                    if !objects.last_mut()?.insert(name) {
                        return None;
                    }
                }
            }
            b'{' => objects.push(HashSet::new()),
            b'}' => {
                objects.pop();
            }
            _ => {}
        }
        i += 1;
    }
    Some(value)
}

/// Validate exact derived paths without requiring a healthy payload (repair is allowed).
pub fn read_receipt(app_root: &Path, env: &crate::ports::EnvLookup) -> Option<Receipt> {
    let path = app_root.join("current/install.json");
    if !valid_path(app_root) || !no_links(&path) {
        return None;
    }
    let v = parse_receipt_without_duplicate_keys(&fs::read(path).ok()?)?;
    if v.get("schemaVersion")?.as_u64()? != 1 {
        return None;
    }
    let string = |k| v.get(k)?.as_str().map(str::to_owned);
    let r = Receipt {
        kind: string("kind")?,
        repo: string("repo")?,
        app_root: string("appRoot")?.into(),
        install_dir: string("installDir")?.into(),
        launcher_path: string("launcherPath")?.into(),
        target: match v.get("target")? {
            Value::Null => None,
            Value::String(t) => Some(t.clone()),
            _ => return None,
        },
        tag: string("tag")?,
        payload_path: string("payloadPath")?.into(),
    };
    if !valid_repo(&r.repo)
        || !canonical_tag(&r.tag)
        || !same_path(&r.app_root, app_root)
        || [
            &r.app_root,
            &r.install_dir,
            &r.launcher_path,
            &r.payload_path,
        ]
        .iter()
        .any(|p| !valid_path(p))
        || same_path(&r.install_dir, &r.app_root)
    {
        return None;
    }
    for home in ["HOME", "USERPROFILE"] {
        if env(home).is_some_and(|h| same_path(&r.app_root, Path::new(&h))) {
            return None;
        }
    }
    // Compare ancestors explicitly so Windows uses ordinal case-insensitive paths too.
    if r.install_dir
        .ancestors()
        .any(|p| same_path(p, &r.app_root.join("current")))
        || [&r.install_dir, &r.app_root].iter().any(|path| {
            path.ancestors().any(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(".install-"))
            })
        })
        || !same_path(
            &r.launcher_path,
            &r.install_dir.join(if cfg!(windows) {
                "envswitch.cmd"
            } else {
                "envswitch"
            }),
        )
        || r.app_root
            .ancestors()
            .any(|p| same_path(p, &r.launcher_path))
    {
        return None;
    }
    let expected = match (r.kind.as_str(), r.target.as_deref()) {
        ("source", None) => r.app_root.join("current/src/cli.ts"),
        ("native", Some(t))
            if [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "x86_64-unknown-linux-musl",
                "aarch64-unknown-linux-musl",
                "x86_64-pc-windows-msvc",
            ]
            .contains(&t)
                && t.ends_with("windows-msvc") == cfg!(windows) =>
        {
            r.app_root.join(format!(
                "current/bin/envswitch_{}_{}{}",
                &r.tag[1..],
                t,
                if cfg!(windows) { ".exe" } else { "" }
            ))
        }
        _ => return None,
    };
    same_path(&r.payload_path, &expected).then_some(r)
}

pub(crate) fn locate(
    current_exe: Option<&Path>,
    app_dir: Option<&Path>,
    env: &crate::ports::EnvLookup,
) -> crate::error::Result<Receipt> {
    use crate::error::Error;
    if app_dir.is_some_and(|p| p.join(".git").exists()) {
        return Err(Error::user(SOURCE_GUIDANCE));
    }
    let exe = current_exe
        .and_then(|p| canonical_path(p).ok())
        .ok_or_else(|| Error::user(RECEIPT_GUIDANCE))?;
    let dir = exe.parent().ok_or_else(|| Error::user(RECEIPT_GUIDANCE))?;
    if matches!(
        dir.file_name().and_then(|n| n.to_str()),
        Some("debug" | "release")
    ) && (dir.parent().is_some_and(|p| p.ends_with("target"))
        || dir
            .parent()
            .and_then(Path::parent)
            .is_some_and(|p| p.ends_with("target")))
    {
        return Err(Error::user(SOURCE_GUIDANCE));
    }
    if dir.join(".git").exists() {
        return Err(Error::user(SOURCE_GUIDANCE));
    }
    let candidate = dir.parent().ok_or_else(|| Error::user(RECEIPT_GUIDANCE))?;
    if candidate.join(".git").exists() {
        return Err(Error::user(SOURCE_GUIDANCE));
    }
    let expected_name = format!(
        "envswitch_{}_{}{}",
        env!("CARGO_PKG_VERSION"),
        env!("TARGET"),
        if cfg!(windows) { ".exe" } else { "" }
    );
    if dir
        .file_name()
        .is_none_or(|n| !same_path(Path::new(n), Path::new("bin")))
        || candidate
            .file_name()
            .is_none_or(|n| !same_path(Path::new(n), Path::new("current")))
        || exe
            .file_name()
            .is_none_or(|n| !same_path(Path::new(n), Path::new(&expected_name)))
    {
        return Err(Error::user(RECEIPT_GUIDANCE));
    }
    let root = app_dir
        .unwrap_or(candidate)
        .parent()
        .ok_or_else(|| Error::user(RECEIPT_GUIDANCE))?;
    if !same_path(candidate, &root.join("current")) {
        return Err(Error::user(RECEIPT_GUIDANCE));
    }
    let receipt = read_receipt(root, env).ok_or_else(|| Error::user(RECEIPT_GUIDANCE))?;
    if receipt.tag == format!("v{}", env!("CARGO_PKG_VERSION"))
        && !same_path(&receipt.payload_path, &exe)
    {
        return Err(Error::user(RECEIPT_GUIDANCE));
    }
    Ok(receipt)
}
