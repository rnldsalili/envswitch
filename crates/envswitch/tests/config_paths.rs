//! Output path safety (contract 02 §2, §8.2 rows 5–7). The string-only
//! decisions are covered exhaustively for both flavors by the unit tests in
//! `src/config/paths/tests.rs`; these tests exercise the real filesystem.

mod support;

use std::fs;
use std::path::Path;

use envswitch::Error;
use envswitch::config::{resolve_output_path, resolve_targets};
use envswitch::model::TargetConfig;

use support::TempProject;

fn user_error(root: &Path, output: &str) -> String {
    match resolve_output_path(root, output) {
        Err(Error::User(message)) => message,
        other => panic!("expected a user error for {output:?}, got {other:?}"),
    }
}

fn target(name: &str, output: &str) -> TargetConfig {
    TargetConfig {
        name: name.to_string(),
        output: output.to_string(),
        groups: None,
    }
}

// --- 02 §8.2 row 5 ---------------------------------------------------------

#[cfg(unix)]
#[test]
fn resolves_target_output_inside_project_root() {
    // The root need not exist.
    assert_eq!(
        resolve_output_path(Path::new("/tmp/project"), "apps/api/.env").unwrap(),
        Path::new("/tmp/project/apps/api/.env")
    );
}

#[test]
fn resolves_target_output_inside_a_temp_root() {
    let project = TempProject::new();
    let resolved = resolve_output_path(project.root(), "apps/api/.env").unwrap();
    assert_eq!(
        resolved,
        project.root().join("apps").join("api").join(".env")
    );
    // Not canonicalized: the root prefix is kept exactly as given.
    assert!(resolved.starts_with(project.root()));
    // Missing directories are fine and nothing is created.
    assert!(!project.path("apps").exists());
}

// --- 02 §8.2 row 6 ---------------------------------------------------------

#[cfg(unix)]
#[test]
fn rejects_output_paths_that_escape_the_project_root() {
    let root = Path::new("/tmp/project");
    let escaped = user_error(root, "../outside/.env");
    assert!(escaped.contains("escapes"));
    assert_eq!(
        escaped,
        "target output path escapes the project root: ../outside/.env"
    );
    let absolute = user_error(root, "/tmp/outside/.env");
    assert!(absolute.contains("relative"));
    assert_eq!(
        absolute,
        "target output path must be relative to the project root: /tmp/outside/.env"
    );
}

#[test]
fn rejects_escapes_and_absolute_paths_on_every_platform() {
    let project = TempProject::new();
    let root = project.root();
    for output in [
        ".",
        "./",
        "a/..",
        "a/../../x",
        "../outside/.env",
        "..env",
        "..foo/.env",
        "...",
    ] {
        assert_eq!(
            user_error(root, output),
            format!("target output path escapes the project root: {output}"),
            "{output:?}"
        );
    }
    // `/x` is absolute for both `path.posix` and `path.win32`.
    assert_eq!(
        user_error(root, "/tmp/outside/.env"),
        "target output path must be relative to the project root: /tmp/outside/.env"
    );
    for output in ["", " ", "\t\r\n", "\u{feff}", "\u{3000}"] {
        assert_eq!(
            user_error(root, output),
            "target output path cannot be empty",
            "{output:?}"
        );
    }
}

#[test]
fn collapses_dot_segments_lexically_before_touching_the_filesystem() {
    let project = TempProject::new();
    let outside = project.outside().join("outside");
    fs::create_dir(&outside).unwrap();
    project.link_dir("linked", &outside).unwrap();
    let root = project.root();
    // `linked/..` collapses to the root before any lstat, even though
    // `linked` is a link.
    assert_eq!(
        resolve_output_path(root, "linked/../x").unwrap(),
        root.join("x")
    );
    assert_eq!(
        resolve_output_path(root, "apps//api/./.env").unwrap(),
        root.join("apps").join("api").join(".env")
    );
    // A trailing separator is dropped (the output becomes a file `api`).
    assert_eq!(
        resolve_output_path(root, "apps/api/").unwrap(),
        root.join("apps").join("api")
    );
    // No tilde expansion.
    assert_eq!(
        resolve_output_path(root, "~/.env").unwrap(),
        root.join("~").join(".env")
    );
    // U+0085 is not JS whitespace, so it is a (strange) file name.
    assert_eq!(
        resolve_output_path(root, "\u{85}").unwrap(),
        root.join("\u{85}")
    );
}

#[cfg(unix)]
#[test]
fn backslashes_are_literal_file_name_characters_on_posix() {
    let project = TempProject::new();
    let root = project.root();
    for output in ["\\\\server\\share\\x", "C:\\x", "a\\..\\..\\x", "C:x"] {
        assert_eq!(
            resolve_output_path(root, output).unwrap(),
            root.join(output),
            "{output:?}"
        );
    }
}

#[test]
fn a_relative_root_resolves_against_the_process_cwd() {
    let resolved =
        resolve_output_path(Path::new("envswitch-relroot-does-not-exist"), "x/.env").unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        resolved,
        cwd.join("envswitch-relroot-does-not-exist")
            .join("x")
            .join(".env")
    );
}

#[test]
fn a_root_with_dot_segments_is_normalized() {
    let project = TempProject::new();
    project.dir("sub");
    let root = project.root().join("sub").join("..");
    assert_eq!(
        resolve_output_path(&root, ".env").unwrap(),
        project.root().join(".env")
    );
}

// --- 02 §8.2 row 7 ---------------------------------------------------------

#[test]
fn rejects_output_paths_with_symlinked_parents() {
    let project = TempProject::new();
    let outside = project.outside().join("outside");
    fs::create_dir(&outside).unwrap();
    project.link_dir("linked", &outside).unwrap();

    let message = user_error(project.root(), "linked/.env");
    assert!(message.contains("symbolic-link parent"));
    assert_eq!(
        message,
        "target output path contains a symbolic-link parent: linked/.env"
    );
    assert!(!outside.join(".env").exists());
}

#[test]
fn rejects_a_link_anywhere_between_the_output_and_the_root() {
    let project = TempProject::new();
    let outside = project.outside().join("outside");
    fs::create_dir_all(outside.join("sub")).unwrap();
    project.link_dir("apps/linked", &outside).unwrap();
    for output in [
        "apps/linked/.env",
        "apps/linked/sub/.env",
        "apps/linked/missing/deeper/.env",
    ] {
        assert_eq!(
            user_error(project.root(), output),
            format!("target output path contains a symbolic-link parent: {output}"),
            "{output}"
        );
    }
}

#[test]
fn rejects_a_root_that_is_itself_a_link() {
    let project = TempProject::new();
    let alias = project.outside().join("alias");
    support::link_dir(project.root(), &alias).unwrap();
    assert_eq!(
        user_error(&alias, "apps/.env"),
        "target output path contains a symbolic-link parent: apps/.env"
    );
    assert_eq!(
        user_error(&alias, ".env"),
        "target output path contains a symbolic-link parent: .env"
    );
}

#[test]
fn links_above_the_root_are_not_checked() {
    let project = TempProject::new();
    let alias = project.outside().join("alias");
    support::link_dir(project.outside(), &alias).unwrap();
    let root = alias.join("envswitch-project");
    assert_eq!(
        resolve_output_path(&root, "apps/.env").unwrap(),
        root.join("apps").join(".env")
    );
}

#[cfg(unix)]
#[test]
fn the_output_itself_may_be_a_symlink() {
    let project = TempProject::new();
    let outside = project.outside().join("real.env");
    fs::write(&outside, "X=1\n").unwrap();
    std::os::unix::fs::symlink(&outside, project.path(".env")).unwrap();
    assert_eq!(
        resolve_output_path(project.root(), ".env").unwrap(),
        project.path(".env")
    );
}

// --- raw filesystem errors (DIVERGENCES d1) --------------------------------

#[cfg(unix)]
#[test]
fn a_file_parent_surfaces_enotdir_as_an_internal_error() {
    let project = TempProject::new();
    project.file("afile", "x");
    let err = resolve_output_path(project.root(), "afile/sub/.env").unwrap_err();
    assert_eq!(
        err,
        Error::Internal(format!(
            "ENOTDIR: not a directory, lstat '{}'",
            project.path("afile/sub").display()
        ))
    );
    assert_eq!(err.exit_code(), 2);
    // The direct child of a file is never lstat'd (only its parents are).
    assert_eq!(
        resolve_output_path(project.root(), "afile/.env").unwrap(),
        project.path("afile/.env")
    );
}

#[cfg(unix)]
#[test]
fn an_unsearchable_parent_surfaces_eacces() {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new();
    project.dir("locked/inner");
    let locked = project.path("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let still_readable = fs::read_dir(&locked).is_ok();
    let result = resolve_output_path(project.root(), "locked/inner/.env");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if still_readable {
        // Running as root: permissions are not enforced.
        return;
    }
    assert_eq!(
        result,
        Err(Error::Internal(format!(
            "EACCES: permission denied, lstat '{}'",
            project.path("locked/inner").display()
        )))
    );
}

/// Deep nesting: Bun rejects a parent of `PATH_MAX` bytes or more before the
/// syscall (`ENAMETOOLONG: name too long, open`, no path, exit 2). Rust must
/// fail on that first parent too, holding one path at a time (the review
/// measured ~75 MB for 8,000 levels before the walk became lazy, and a
/// 128 KiB path would have needed ~4 GiB).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn deep_nesting_fails_on_the_first_parent_like_bun() {
    let project = TempProject::new();
    for levels in [8_000, 65_536] {
        let output = format!("{}.env", "a/".repeat(levels));
        let err = resolve_output_path(project.root(), &output).unwrap_err();
        assert_eq!(
            err,
            Error::internal("ENAMETOOLONG: name too long, open"),
            "{levels}"
        );
        assert_eq!(err.exit_code(), 2);
    }
    // The length check runs before the NUL check.
    let output = format!("{}\u{0}/.env", "a".repeat(5_000));
    assert_eq!(
        resolve_output_path(project.root(), &output),
        Err(Error::internal("ENAMETOOLONG: name too long, open"))
    );
}

#[cfg(unix)]
#[test]
fn an_os_enametoolong_reports_the_lstat_path() {
    // Under PATH_MAX, but one component exceeds NAME_MAX (255).
    let project = TempProject::new();
    let long = "x".repeat(300);
    let err = resolve_output_path(project.root(), &format!("{long}/.env")).unwrap_err();
    assert_eq!(
        err,
        Error::Internal(format!(
            "ENAMETOOLONG: name too long, lstat '{}'",
            project.path(&long).display()
        ))
    );
    assert_eq!(err.exit_code(), 2);
}

#[cfg(windows)]
#[test]
fn deep_nesting_terminates_on_windows() {
    // Bun's Windows limit is unverified; the walk must still finish and
    // never report a link or other user error.
    let project = TempProject::new();
    let output = format!("{}.env", "a/".repeat(8_000));
    match resolve_output_path(project.root(), &output) {
        Ok(_) | Err(Error::Internal(_)) => {}
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_nul_byte_in_a_parent_is_rejected_like_bun() {
    let project = TempProject::new();
    let err = resolve_output_path(project.root(), "a\u{0}b/.env").unwrap_err();
    let parent = project.root().join("a\u{0}b");
    let received = serde_json::Value::String(parent.to_string_lossy().into_owned());
    assert_eq!(
        err,
        Error::Internal(format!(
            "The argument 'path' must be a string, Uint8Array, or URL without null bytes. Received {received}"
        ))
    );
    assert_eq!(err.exit_code(), 2);
}

// --- resolve_targets -------------------------------------------------------

#[test]
fn resolve_targets_keeps_order_and_stops_at_the_first_failure() {
    let project = TempProject::new();
    let root = project.root();
    let targets = [
        target("api", "apps/api/.env"),
        target("web", "apps/web/.env"),
    ];
    let resolved = resolve_targets(root, &targets).unwrap();
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].target, targets[0]);
    assert_eq!(
        resolved[0].output_path,
        root.join("apps").join("api").join(".env")
    );
    assert_eq!(resolved[1].target, targets[1]);
    assert_eq!(
        resolved[1].output_path,
        root.join("apps").join("web").join(".env")
    );

    let failing = [target("api", "."), target("web", "../x"), target("ok", "x")];
    assert_eq!(
        resolve_targets(root, &failing),
        Err(Error::user(
            "target output path escapes the project root: ."
        ))
    );
    assert_eq!(resolve_targets(root, &[]), Ok(Vec::new()));
}

// --- Windows (path.win32 rules) --------------------------------------------

#[cfg(windows)]
mod windows {
    use super::*;

    /// The root's drive letter as written (`C`).
    fn drive(root: &Path) -> char {
        let text = root.to_string_lossy();
        let mut chars = text.chars();
        let letter = chars.next().unwrap();
        assert_eq!(
            chars.next(),
            Some(':'),
            "temp root must be on a drive: {text}"
        );
        letter
    }

    #[test]
    fn rooted_drive_and_unc_paths_are_absolute() {
        let project = TempProject::new();
        for output in [
            "\\foo",
            "/foo",
            "C:\\x",
            "C:/x",
            "c:\\x",
            "\\\\srv\\sh\\x",
            "//srv/sh/x",
            "\\\\?\\C:\\x",
            "\\\\.\\x",
        ] {
            assert_eq!(
                user_error(project.root(), output),
                format!("target output path must be relative to the project root: {output}"),
                "{output:?}"
            );
        }
    }

    #[test]
    fn forward_and_back_slashes_both_separate() {
        let project = TempProject::new();
        let expected = project.root().join("apps").join("api").join(".env");
        assert_eq!(
            resolve_output_path(project.root(), "apps/api/.env").unwrap(),
            expected
        );
        assert_eq!(
            resolve_output_path(project.root(), "apps\\api\\.env").unwrap(),
            expected
        );
        assert_eq!(
            user_error(project.root(), "a\\..\\..\\x"),
            "target output path escapes the project root: a\\..\\..\\x"
        );
    }

    #[test]
    fn drive_relative_paths_keep_parity() {
        let project = TempProject::new();
        let root = project.root();
        let letter = drive(root);
        // Same drive: resolved under the root (Node `path.win32` behavior).
        let same = format!("{letter}:x");
        assert_eq!(
            resolve_output_path(root, &same).unwrap().to_string_lossy(),
            root.join("x").to_string_lossy()
        );
        // DIVERGENCES i1: same drive, other letter case. TS passes the escape
        // check, then spins forever in its lstat loop; Rust stops at the root
        // and accepts.
        let lower = format!("{}:x", letter.to_ascii_lowercase());
        let resolved = resolve_output_path(root, &lower).unwrap();
        assert!(
            resolved
                .to_string_lossy()
                .eq_ignore_ascii_case(&root.join("x").to_string_lossy()),
            "{resolved:?}"
        );
        // `C:` alone is the root itself.
        assert_eq!(
            user_error(root, &format!("{letter}:")),
            format!("target output path escapes the project root: {letter}:")
        );
        // Another drive escapes.
        let other = if letter.eq_ignore_ascii_case(&'Q') {
            'R'
        } else {
            'Q'
        };
        let other_output = format!("{other}:x");
        assert_eq!(
            user_error(root, &other_output),
            format!("target output path escapes the project root: {other_output}")
        );
        assert_eq!(
            user_error(root, &format!("{letter}:..\\x")),
            format!("target output path escapes the project root: {letter}:..\\x")
        );
    }

    #[test]
    fn a_file_parent_is_reported_as_missing_on_windows() {
        // Windows reports ERROR_PATH_NOT_FOUND, which libuv maps to ENOENT,
        // so the walk continues.
        let project = TempProject::new();
        project.file("afile", "x");
        assert_eq!(
            resolve_output_path(project.root(), "afile/sub/.env").unwrap(),
            project.root().join("afile").join("sub").join(".env")
        );
    }

    #[test]
    fn rejects_a_junction_parent() {
        let project = TempProject::new();
        let outside = project.outside().join("outside");
        fs::create_dir(&outside).unwrap();
        junction::create(&outside, project.path("linked")).unwrap();
        assert_eq!(
            user_error(project.root(), "linked\\.env"),
            "target output path contains a symbolic-link parent: linked\\.env"
        );
        assert!(!outside.join(".env").exists());
    }
}
