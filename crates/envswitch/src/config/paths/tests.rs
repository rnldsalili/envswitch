//! Unit tests for the Node `path` port and the lexical half of
//! `resolve_output_path`, for both flavors on every host.
//!
//! Every expected value in the tables below was produced by Bun 1.4.0's
//! `node:path` (`path.posix` / `path.win32`) and the TS
//! `resolveOutputPath` steps from `config.ts:106-135`. During development
//! the port was also checked against two 4,000-string fuzz corpora
//! (normalize, dirname, basename, join, isAbsolute), 24,000 resolve and
//! relative pairs, and 128,000 full output-path decisions, with no
//! differences.

use super::node::{posix, win32};
use super::{Flavor, Lexical};
use crate::error::{Error, Result};

#[derive(Debug)]
enum Decision {
    /// Accepted: `(outputAbs, directories the lstat walk visits)`.
    Accept(&'static str, &'static [&'static str]),
    /// Rejected with this `Error::User` message.
    Reject(&'static str),
    /// Accepted lexically, but TS loops forever in the lstat walk (it never
    /// reaches a byte-equal root). `(outputAbs, case-insensitive root)`.
    Loops(&'static str, &'static str),
}
use Decision::{Accept, Loops, Reject};

// posix: (input, isAbsolute, normalize, dirname, basename)
const POSIX_SINGLE: &[(&str, bool, &str, &str, &str)] = &[
    ("", false, ".", ".", ""),
    (".", false, ".", ".", "."),
    ("./", false, "./", ".", "."),
    ("/", true, "/", "/", ""),
    ("//", true, "/", "/", ""),
    ("///", true, "/", "/", ""),
    ("a/..", false, ".", "a", ".."),
    ("a/../..", false, "..", "a/..", ".."),
    ("/..", true, "/", "/", ".."),
    ("//a//b/", true, "/a/b/", "//a/", "b"),
    (
        "apps//api/./.env",
        false,
        "apps/api/.env",
        "apps//api/.",
        ".env",
    ),
    ("..env", false, "..env", ".", "..env"),
    ("~/.env", false, "~/.env", "~", ".env"),
    ("C:\\x", false, "C:\\x", ".", "C:\\x"),
    ("/a/b/..", true, "/a", "/a/b", ".."),
    ("a//", false, "a/", ".", "a"),
    ("a/b/", false, "a/b/", "a", "b"),
    (
        "/tmp/\u{130}/\u{e9}/",
        true,
        "/tmp/\u{130}/\u{e9}/",
        "/tmp/\u{130}",
        "\u{e9}",
    ),
];
// win32: (input, isAbsolute, normalize, dirname, basename)
const WIN32_SINGLE: &[(&str, bool, &str, &str, &str)] = &[
    ("", false, ".", ".", ""),
    (".", false, ".", ".", "."),
    ("/", true, "\\", "/", ""),
    ("\\", true, "\\", "\\", ""),
    ("C:", false, "C:.", "C:", ""),
    ("C:x", false, "C:x", "C:", "x"),
    ("c:x", false, "c:x", "c:", "x"),
    ("C:\\", true, "C:\\", "C:\\", ""),
    ("c:\\", true, "c:\\", "c:\\", ""),
    ("C:/x/../y", true, "C:\\y", "C:/x/..", "y"),
    ("a/../C:x", false, "C:x", "a/..", "C:x"),
    ("\\\\srv\\sh", true, "\\\\srv\\sh\\", "\\\\srv\\sh", "sh"),
    (
        "\\\\srv\\sh\\",
        true,
        "\\\\srv\\sh\\",
        "\\\\srv\\sh\\",
        "sh",
    ),
    (
        "\\\\srv\\sh\\x\\..\\..",
        true,
        "\\\\srv\\sh\\",
        "\\\\srv\\sh\\x\\..",
        "..",
    ),
    ("\\\\srv", true, "\\srv", "\\", "srv"),
    ("//srv/sh/x/", true, "\\\\srv\\sh\\x\\", "//srv/sh/", "x"),
    ("\\\\?\\C:\\x", true, "\\\\?\\C:\\x", "\\\\?\\C:\\", "x"),
    ("\\\\.\\x", true, "\\\\.\\x\\", "\\\\.\\x", "x"),
    ("a\\..\\..\\x", false, "..\\x", "a\\..\\..", "x"),
    ("C:..\\x", false, "C:..\\x", "C:..", "x"),
    ("a/b/", false, "a\\b\\", "a", "b"),
    ("..env", false, "..env", ".", "..env"),
    ("CON:x", false, "CON:x", ".", "CON:x"),
    ("C:foo", false, "C:foo", "C:", "foo"),
    ("C:\\a\\b\\", true, "C:\\a\\b\\", "C:\\a", "b"),
    ("\\x", true, "\\x", "\\", "x"),
    ("\u{c9}:\\x", false, "\u{c9}:\\x", "\u{c9}:", "x"),
    (
        "\\\\srv\\sh\\x\\y",
        true,
        "\\\\srv\\sh\\x\\y",
        "\\\\srv\\sh\\x",
        "y",
    ),
];
// posix: (a, b, resolve(a, b))
const POSIX_RESOLVE: &[(&str, &str, &str)] = &[
    (
        "/tmp/project",
        "apps/api/.env",
        "/tmp/project/apps/api/.env",
    ),
    ("/tmp/project/", "../x", "/tmp/x"),
    ("/", "..", "/"),
    ("//a", "b", "/a/b"),
    ("/a", "/b", "/b"),
    ("/a/b/..", "c", "/a/c"),
    ("/a", "./", "/a"),
];
// win32: (a, b, resolve(a, b))
const WIN32_RESOLVE: &[(&str, &str, &str)] = &[
    ("C:\\Proj", "x", "C:\\Proj\\x"),
    ("C:\\Proj", "..\\x", "C:\\x"),
    ("C:\\Proj", "c:x", "c:\\Proj\\x"),
    ("C:\\Proj", "C:..\\x", "C:\\x"),
    ("C:\\Proj", "\\x", "C:\\x"),
    ("C:\\Proj", "a/./b//c/", "C:\\Proj\\a\\b\\c"),
    ("\\\\srv\\share", "x", "\\\\srv\\share\\x"),
    ("C:/Proj/", "x", "C:\\Proj\\x"),
    ("C:\\Proj", "C:", "C:\\Proj"),
    ("\\\\srv\\share\\", "..\\..", "\\\\srv\\share\\"),
    ("C:\\", "..", "C:\\"),
];
// posix: (from, to, relative(from, to))
const POSIX_RELATIVE: &[(&str, &str, &str)] = &[
    ("/a", "/a/b", "b"),
    ("/a/b", "/a", ".."),
    ("/", "/a", "a"),
    ("/a", "/b", "../b"),
    ("/a/b", "/a/bc", "../bc"),
    ("/a", "/a", ""),
    ("/a/b", "/", "../.."),
    ("/a/bc", "/a/b", "../b"),
    ("/p", "/p/..env", "..env"),
];
// win32: (from, to, relative(from, to))
const WIN32_RELATIVE: &[(&str, &str, &str)] = &[
    ("C:\\Proj", "C:\\Proj\\x", "x"),
    ("C:\\Proj", "c:\\proj\\x", "x"),
    ("C:\\", "C:\\x", "x"),
    ("C:\\Proj\\a", "C:\\Proj", ".."),
    ("C:\\Proj", "D:\\x", "D:\\x"),
    ("C:\\\u{c9}\\p", "C:\\\u{e9}\\p\\x", "..\\..\\\u{e9}\\p\\x"),
    ("\\\\srv\\sh\\p", "\\\\srv\\sh\\p\\x", "x"),
    ("C:\\Proj", "C:\\Projx", "..\\Projx"),
    ("C:\\Proj", "C:\\Proj\\..env", "..env"),
    ("C:\\a\\b", "C:\\", "..\\.."),
    ("\\\\srv\\sh\\", "\\\\srv\\sh\\x", "x"),
];
// posix: (root, join(root, "package.json"))
const POSIX_JOIN: &[(&str, &str)] = &[
    ("/tmp/p", "/tmp/p/package.json"),
    ("/tmp/p/", "/tmp/p/package.json"),
    ("", "package.json"),
    ("rel", "rel/package.json"),
    ("/a/../b", "/b/package.json"),
    ("//x", "/x/package.json"),
];
// win32: (root, join(root, "package.json"))
const WIN32_JOIN: &[(&str, &str)] = &[
    ("C:\\Proj", "C:\\Proj\\package.json"),
    ("C:\\Proj\\", "C:\\Proj\\package.json"),
    ("\\\\srv\\share", "\\\\srv\\share\\package.json"),
    ("//srv", "\\\\srv\\package.json\\"),
    ("C:", "C:\\package.json"),
    ("", "package.json"),
    ("\\\\\\x", "\\x\\package.json"),
    ("C:/a/../b", "C:\\b\\package.json"),
];
// posix: (root, output, decision)
const POSIX_DECISIONS: &[(&str, &str, Decision)] = &[
    (
        "/tmp/project",
        "apps/api/.env",
        Accept(
            "/tmp/project/apps/api/.env",
            &["/tmp/project/apps/api", "/tmp/project/apps", "/tmp/project"],
        ),
    ),
    (
        "/tmp/project",
        "./.env",
        Accept("/tmp/project/.env", &["/tmp/project"]),
    ),
    (
        "/tmp/project",
        "apps//api/./.env",
        Accept(
            "/tmp/project/apps/api/.env",
            &["/tmp/project/apps/api", "/tmp/project/apps", "/tmp/project"],
        ),
    ),
    (
        "/tmp/project",
        "apps/api/",
        Accept(
            "/tmp/project/apps/api",
            &["/tmp/project/apps", "/tmp/project"],
        ),
    ),
    (
        "/tmp/project",
        ".",
        Reject("target output path escapes the project root: ."),
    ),
    (
        "/tmp/project",
        "./",
        Reject("target output path escapes the project root: ./"),
    ),
    (
        "/tmp/project",
        "a/..",
        Reject("target output path escapes the project root: a/.."),
    ),
    (
        "/tmp/project",
        "a/../../x",
        Reject("target output path escapes the project root: a/../../x"),
    ),
    (
        "/tmp/project",
        "..env",
        Reject("target output path escapes the project root: ..env"),
    ),
    (
        "/tmp/project",
        "..foo/.env",
        Reject("target output path escapes the project root: ..foo/.env"),
    ),
    (
        "/tmp/project",
        "/abs",
        Reject("target output path must be relative to the project root: /abs"),
    ),
    (
        "/tmp/project",
        "~/.env",
        Accept("/tmp/project/~/.env", &["/tmp/project/~", "/tmp/project"]),
    ),
    (
        "/tmp/project",
        "\\\\server\\share\\x",
        Accept("/tmp/project/\\\\server\\share\\x", &["/tmp/project"]),
    ),
    (
        "/tmp/project",
        "C:\\x",
        Accept("/tmp/project/C:\\x", &["/tmp/project"]),
    ),
    (
        "/tmp/project",
        "a\\..\\..\\x",
        Accept("/tmp/project/a\\..\\..\\x", &["/tmp/project"]),
    ),
    (
        "/tmp/project",
        " ",
        Reject("target output path cannot be empty"),
    ),
    (
        "/tmp/project",
        "\u{feff}",
        Reject("target output path cannot be empty"),
    ),
    (
        "/tmp/project",
        "\u{85}",
        Accept("/tmp/project/\u{85}", &["/tmp/project"]),
    ),
    (
        "/tmp/project",
        "linked/../x",
        Accept("/tmp/project/x", &["/tmp/project"]),
    ),
    (
        "/tmp/project",
        "...",
        Reject("target output path escapes the project root: ..."),
    ),
    (
        "/tmp/project",
        ".../x",
        Reject("target output path escapes the project root: .../x"),
    ),
    (
        "/tmp/project",
        "x/..",
        Reject("target output path escapes the project root: x/.."),
    ),
    ("/", "x", Accept("/x", &["/"])),
    ("/", "a/b", Accept("/a/b", &["/a", "/"])),
    (
        "/tmp/project/",
        "x",
        Accept("/tmp/project/x", &["/tmp/project"]),
    ),
    (
        "//tmp//project",
        "a/b",
        Accept("/tmp/project/a/b", &["/tmp/project/a", "/tmp/project"]),
    ),
];
// win32: (root, output, decision)
const WIN32_DECISIONS: &[(&str, &str, Decision)] = &[
    (
        "C:\\Proj",
        "apps\\api\\.env",
        Accept(
            "C:\\Proj\\apps\\api\\.env",
            &["C:\\Proj\\apps\\api", "C:\\Proj\\apps", "C:\\Proj"],
        ),
    ),
    (
        "C:\\Proj",
        "apps/api/.env",
        Accept(
            "C:\\Proj\\apps\\api\\.env",
            &["C:\\Proj\\apps\\api", "C:\\Proj\\apps", "C:\\Proj"],
        ),
    ),
    (
        "C:\\Proj",
        ".",
        Reject("target output path escapes the project root: ."),
    ),
    (
        "C:\\Proj",
        "..env",
        Reject("target output path escapes the project root: ..env"),
    ),
    (
        "C:\\Proj",
        "..\\x",
        Reject("target output path escapes the project root: ..\\x"),
    ),
    ("C:\\Proj", "C:x", Accept("C:\\Proj\\x", &["C:\\Proj"])),
    ("C:\\Proj", "c:x", Loops("c:\\Proj\\x", "c:\\Proj")),
    (
        "C:\\Proj",
        "C:",
        Reject("target output path escapes the project root: C:"),
    ),
    (
        "C:\\Proj",
        "D:x",
        Reject("target output path escapes the project root: D:x"),
    ),
    (
        "C:\\Proj",
        "\\x",
        Reject("target output path must be relative to the project root: \\x"),
    ),
    (
        "C:\\Proj",
        "/x",
        Reject("target output path must be relative to the project root: /x"),
    ),
    (
        "C:\\Proj",
        "C:\\x",
        Reject("target output path must be relative to the project root: C:\\x"),
    ),
    (
        "C:\\Proj",
        "C:/x",
        Reject("target output path must be relative to the project root: C:/x"),
    ),
    (
        "C:\\Proj",
        "\\\\srv\\sh\\x",
        Reject("target output path must be relative to the project root: \\\\srv\\sh\\x"),
    ),
    (
        "C:\\Proj",
        "//srv/sh/x",
        Reject("target output path must be relative to the project root: //srv/sh/x"),
    ),
    (
        "C:\\Proj",
        "\\\\?\\C:\\x",
        Reject("target output path must be relative to the project root: \\\\?\\C:\\x"),
    ),
    (
        "C:\\Proj",
        "C:..\\x",
        Reject("target output path escapes the project root: C:..\\x"),
    ),
    ("C:\\Proj", "a/../C:x", Accept("C:\\Proj\\x", &["C:\\Proj"])),
    ("C:\\Proj", "CON", Accept("C:\\Proj\\CON", &["C:\\Proj"])),
    (
        "C:\\Proj",
        "a\\..\\..\\x",
        Reject("target output path escapes the project root: a\\..\\..\\x"),
    ),
    (
        "C:\\Proj",
        "\u{3000}",
        Reject("target output path cannot be empty"),
    ),
    (
        "C:\\Proj",
        "x:y",
        Reject("target output path escapes the project root: x:y"),
    ),
    (
        "C:\\Proj",
        "a:b/c",
        Reject("target output path escapes the project root: a:b/c"),
    ),
    (
        "\\\\srv\\share\\proj",
        ".env",
        Accept("\\\\srv\\share\\proj\\.env", &["\\\\srv\\share\\proj"]),
    ),
    (
        "\\\\srv\\share\\proj",
        "a\\b",
        Accept(
            "\\\\srv\\share\\proj\\a\\b",
            &["\\\\srv\\share\\proj\\a", "\\\\srv\\share\\proj"],
        ),
    ),
    (
        "\\\\srv\\share",
        "x",
        Accept("\\\\srv\\share\\x", &["\\\\srv\\share\\"]),
    ),
    ("C:\\", "x", Accept("C:\\x", &["C:\\"])),
    ("C:\\", "a/b", Accept("C:\\a\\b", &["C:\\a", "C:\\"])),
    ("C:/Proj/", "x", Accept("C:\\Proj\\x", &["C:\\Proj"])),
];

fn posix_cwd() -> Result<Vec<u8>> {
    Ok(b"/cwd".to_vec())
}

fn win32_cwd() -> Result<Vec<u8>> {
    Ok(b"C:\\cwd".to_vec())
}

fn no_drive_cwd(_: &[u8]) -> Option<Vec<u8>> {
    None
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

/// Every directory `walk_parents` visits, in order.
fn walked(flavor: Flavor, lexical: &Lexical) -> Vec<String> {
    let mut visited = Vec::new();
    flavor
        .walk_parents(lexical, |dir| {
            visited.push(text(dir));
            Ok(())
        })
        .unwrap();
    visited
}

fn cwd_for(flavor: Flavor) -> fn() -> Result<Vec<u8>> {
    match flavor {
        Flavor::Posix => posix_cwd,
        Flavor::Win32 => win32_cwd,
    }
}

#[test]
fn single_argument_functions_match_bun() {
    for (flavor, table) in [(Flavor::Posix, POSIX_SINGLE), (Flavor::Win32, WIN32_SINGLE)] {
        for &(input, absolute, normalize, dirname, basename) in table {
            let bytes = input.as_bytes();
            assert_eq!(
                flavor.is_absolute(bytes),
                absolute,
                "{flavor:?}.isAbsolute({input:?})"
            );
            assert_eq!(
                text(&flavor.normalize(bytes)),
                normalize,
                "{flavor:?}.normalize({input:?})"
            );
            assert_eq!(
                text(&flavor.dirname(bytes)),
                dirname,
                "{flavor:?}.dirname({input:?})"
            );
            assert_eq!(
                text(&flavor.basename(bytes)),
                basename,
                "{flavor:?}.basename({input:?})"
            );
        }
    }
}

#[test]
fn resolve_matches_bun() {
    for (flavor, table) in [
        (Flavor::Posix, POSIX_RESOLVE),
        (Flavor::Win32, WIN32_RESOLVE),
    ] {
        for &(a, b, expected) in table {
            let got = flavor
                .resolve(
                    &[a.as_bytes(), b.as_bytes()],
                    &cwd_for(flavor),
                    &no_drive_cwd,
                )
                .unwrap();
            assert_eq!(text(&got), expected, "{flavor:?}.resolve({a:?}, {b:?})");
        }
    }
}

#[test]
fn relative_matches_bun() {
    for (flavor, table) in [
        (Flavor::Posix, POSIX_RELATIVE),
        (Flavor::Win32, WIN32_RELATIVE),
    ] {
        for &(from, to, expected) in table {
            let got = flavor
                .relative(
                    from.as_bytes(),
                    to.as_bytes(),
                    &cwd_for(flavor),
                    &no_drive_cwd,
                )
                .unwrap();
            assert_eq!(
                text(&got),
                expected,
                "{flavor:?}.relative({from:?}, {to:?})"
            );
        }
    }
}

#[test]
fn join_matches_bun() {
    for (flavor, table) in [(Flavor::Posix, POSIX_JOIN), (Flavor::Win32, WIN32_JOIN)] {
        for &(root, expected) in table {
            let got = flavor.join(&[root.as_bytes(), b"package.json"]);
            assert_eq!(
                text(&got),
                expected,
                "{flavor:?}.join({root:?}, \"package.json\")"
            );
        }
    }
}

#[test]
fn output_path_decisions_match_ts() {
    for (flavor, table) in [
        (Flavor::Posix, POSIX_DECISIONS),
        (Flavor::Win32, WIN32_DECISIONS),
    ] {
        for (root, output, decision) in table {
            let got =
                flavor.resolve_lexically(root.as_bytes(), output, &cwd_for(flavor), &no_drive_cwd);
            let context = format!("{flavor:?} root {root:?} output {output:?}");
            match decision {
                Accept(output_abs, chain) => {
                    let lexical = got.unwrap_or_else(|err| panic!("{context}: {err}"));
                    assert_eq!(text(&lexical.output_abs), *output_abs, "{context}");
                    assert_eq!(walked(flavor, &lexical), *chain, "{context}");
                }
                Reject(message) => {
                    assert_eq!(got, Err(Error::User(message.to_string())), "{context}");
                }
                Loops(output_abs, stop) => {
                    // DIVERGENCES i1: TS never terminates; Rust stops at the
                    // case-insensitive root and accepts.
                    let lexical = got.unwrap_or_else(|err| panic!("{context}: {err}"));
                    assert_eq!(text(&lexical.output_abs), *output_abs, "{context}");
                    assert_eq!(walked(flavor, &lexical), [*stop], "{context}");
                }
            }
        }
    }
}

#[test]
fn win32_drive_relative_output_on_another_drive_uses_that_drives_cwd() {
    // Node reads `process.env["=D:"]` first, then falls back to `D:\`.
    let with_env = |device: &[u8]| (device == b"D:").then(|| b"D:\\work".to_vec());
    let resolved = win32::resolve(&[b"C:\\Proj", b"D:x"], &win32_cwd, &with_env).unwrap();
    assert_eq!(text(&resolved), "D:\\work\\x");
    let resolved = win32::resolve(&[b"C:\\Proj", b"D:x"], &win32_cwd, &no_drive_cwd).unwrap();
    assert_eq!(text(&resolved), "D:\\x");
    // Either way the output escapes the root.
    let decision = Flavor::Win32.resolve_lexically(b"C:\\Proj", "D:x", &win32_cwd, &with_env);
    assert_eq!(
        decision,
        Err(Error::User(
            "target output path escapes the project root: D:x".to_string()
        ))
    );
}

#[test]
fn relative_roots_resolve_against_the_cwd() {
    let lexical = Flavor::Posix
        .resolve_lexically(b"relroot", "a/.env", &posix_cwd, &no_drive_cwd)
        .unwrap();
    assert_eq!(
        lexical,
        Lexical {
            root_abs: b"/cwd/relroot".to_vec(),
            output_abs: b"/cwd/relroot/a/.env".to_vec(),
        }
    );
    let lexical = Flavor::Win32
        .resolve_lexically(b"relroot", "a/.env", &win32_cwd, &no_drive_cwd)
        .unwrap();
    assert_eq!(text(&lexical.output_abs), "C:\\cwd\\relroot\\a\\.env");
    assert_eq!(text(&posix::resolve(&[b""], &posix_cwd).unwrap()), "/cwd");
}

#[test]
fn cwd_is_only_read_when_needed_and_its_error_propagates() {
    let failing =
        || -> Result<Vec<u8>> { Err(Error::internal("ENOENT: no such file or directory, uv_cwd")) };
    // Absolute roots never consult the cwd.
    assert!(
        Flavor::Posix
            .resolve_lexically(b"/r", "x", &failing, &no_drive_cwd)
            .is_ok()
    );
    assert!(
        Flavor::Win32
            .resolve_lexically(b"C:\\r", "x", &failing, &no_drive_cwd)
            .is_ok()
    );
    // A relative root does, and the error surfaces unchanged (exit 2).
    let err = Flavor::Posix
        .resolve_lexically(b"rel", "x", &failing, &no_drive_cwd)
        .unwrap_err();
    assert_eq!(
        err,
        Error::internal("ENOENT: no such file or directory, uv_cwd")
    );
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn checks_run_in_ts_order() {
    // Emptiness before absoluteness before escape.
    for flavor in [Flavor::Posix, Flavor::Win32] {
        let cwd = cwd_for(flavor);
        let root: &[u8] = if flavor == Flavor::Posix {
            b"/r"
        } else {
            b"C:\\r"
        };
        let message = |output: &str| {
            flavor
                .resolve_lexically(root, output, &cwd, &no_drive_cwd)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(message(""), "target output path cannot be empty");
        assert_eq!(message(" \t\n"), "target output path cannot be empty");
        assert_eq!(
            message("/.."),
            "target output path must be relative to the project root: /.."
        );
        assert_eq!(
            message(".."),
            "target output path escapes the project root: .."
        );
    }
}

#[test]
fn walk_stops_at_a_dirname_fixed_point() {
    // Defensive: never reachable from `resolve_lexically`, but the walk must
    // terminate even for a root that is not an ancestor.
    let lexical = Lexical {
        root_abs: b"/elsewhere".to_vec(),
        output_abs: b"/a/b".to_vec(),
    };
    assert_eq!(walked(Flavor::Posix, &lexical), ["/a", "/"]);
    let lexical = Lexical {
        root_abs: b"D:\\x".to_vec(),
        output_abs: b"C:\\a\\b".to_vec(),
    };
    assert_eq!(walked(Flavor::Win32, &lexical), ["C:\\a", "C:\\"]);
}

#[test]
fn walk_is_lazy_and_stops_at_the_first_error() {
    // 8,000 levels: the walk visits the deepest parent first and stops on
    // its error without computing any other ancestor.
    for flavor in [Flavor::Posix, Flavor::Win32] {
        let root: &[u8] = if flavor == Flavor::Posix {
            b"/r"
        } else {
            b"C:\\r"
        };
        let output = format!("{}.env", "a/".repeat(8000));
        let lexical = flavor
            .resolve_lexically(root, &output, &cwd_for(flavor), &no_drive_cwd)
            .unwrap();
        let mut visits = Vec::new();
        let result = flavor.walk_parents(&lexical, |dir| {
            visits.push(dir.len());
            Err(Error::internal("stop"))
        });
        assert_eq!(result, Err(Error::internal("stop")), "{flavor:?}");
        assert_eq!(
            visits,
            [lexical.output_abs.len() - b"/.env".len()],
            "{flavor:?}"
        );

        // Without errors it visits every level once, root last.
        let mut count = 0usize;
        let mut last = Vec::new();
        flavor
            .walk_parents(&lexical, |dir| {
                count += 1;
                last = dir.to_vec();
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 8001, "{flavor:?}");
        assert_eq!(last, root, "{flavor:?}");
    }
}
