//! Project root markers (contract 02 §1.6, §8.2 row 9). Expected messages
//! were confirmed against the TS `validateProjectRoot` on a real temp tree.

mod support;

use std::fs;
use std::path::Path;

use envswitch::Error;
use envswitch::config::{parse_config, validate_project_root};
use envswitch::model::{Config, ProjectConfig, RootMarker};
use serde_json::{Value, json};

use support::TempProject;

fn config_with_markers(markers: Value) -> Config {
    parse_config(
        &json!({
            "project": { "id": "demo-suite", "rootMarkers": markers },
            "targets": [{ "name": "api", "output": "apps/api/.env" }],
        })
        .to_string(),
    )
    .unwrap()
}

fn check(root: &Path, markers: Value) -> envswitch::Result<()> {
    validate_project_root(&config_with_markers(markers), root)
}

fn user_error(root: &Path, markers: Value) -> String {
    match check(root, markers.clone()) {
        Err(Error::User(message)) => message,
        other => panic!("expected a user error for {markers}, got {other:?}"),
    }
}

// --- 02 §8.2 row 9 ---------------------------------------------------------

#[test]
fn validates_directory_and_package_root_markers() {
    let project = TempProject::named("envswitch-root");
    project.dir("services");
    project.file("package.json", &json!({ "name": "demo-suite" }).to_string());
    let config = parse_config(
        &json!({
            "project": {
                "id": "demo-suite",
                "rootMarkers": [
                    { "path": "services" },
                    { "packageJsonName": "demo-suite" },
                ],
            },
            "targets": [{ "name": "api", "output": "apps/api/.env" }],
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(validate_project_root(&config, project.root()), Ok(()));
}

// --- no markers ------------------------------------------------------------

#[test]
fn nothing_to_check_without_markers() {
    let missing = Path::new("/definitely/not/a/real/root");
    let no_project = parse_config(r#"{"targets":[{"name":"api","output":"x"}]}"#).unwrap();
    assert_eq!(validate_project_root(&no_project, missing), Ok(()));
    let no_markers =
        parse_config(r#"{"project":{"id":"demo"},"targets":[{"name":"api","output":"x"}]}"#)
            .unwrap();
    assert_eq!(validate_project_root(&no_markers, missing), Ok(()));
    assert_eq!(check(missing, json!([])), Ok(()));
}

// --- path markers ----------------------------------------------------------

#[test]
fn path_markers() {
    let project = TempProject::new();
    project.dir("services").file("afile", "x");
    let root = project.root();

    // Accepted: directories, files, trailing slashes, lexical `..`.
    for marker in ["services", "services/", "afile", "missing/../services"] {
        assert_eq!(check(root, json!([{ "path": marker }])), Ok(()), "{marker}");
    }
    assert_eq!(
        user_error(root, json!([{ "path": "missing" }])),
        "project root marker path does not exist: missing"
    );
    // A non-directory parent of the marker itself is never lstat'd; `stat`
    // fails, so it is "missing".
    assert_eq!(
        user_error(root, json!([{ "path": "afile/sub" }])),
        "project root marker path does not exist: afile/sub"
    );
}

#[test]
fn path_markers_reuse_target_flavored_messages() {
    // 02 §9.3 #20.
    let project = TempProject::new();
    let root = project.root();
    assert_eq!(
        user_error(root, json!([{ "path": "." }])),
        "target output path escapes the project root: ."
    );
    assert_eq!(
        user_error(root, json!([{ "path": "../outside" }])),
        "target output path escapes the project root: ../outside"
    );
    assert_eq!(
        user_error(root, json!([{ "path": "/abs" }])),
        "target output path must be relative to the project root: /abs"
    );
}

#[test]
fn path_markers_reject_linked_parents_but_may_be_links_themselves() {
    let project = TempProject::new();
    project.dir("services");
    let outside = project.outside().join("outside");
    fs::create_dir(&outside).unwrap();
    project.link_dir("linked", &outside).unwrap();
    project
        .link_dir("linkdir", &project.path("services"))
        .unwrap();
    let root = project.root();

    assert_eq!(
        user_error(root, json!([{ "path": "linked/x" }])),
        "target output path contains a symbolic-link parent: linked/x"
    );
    assert_eq!(check(root, json!([{ "path": "linkdir" }])), Ok(()));

    // A root that is itself a link fails for any path marker.
    let alias = project.outside().join("alias");
    support::link_dir(root, &alias).unwrap();
    assert_eq!(
        user_error(&alias, json!([{ "path": "services" }])),
        "target output path contains a symbolic-link parent: services"
    );
}

#[cfg(unix)]
#[test]
fn a_dangling_marker_link_counts_as_missing() {
    let project = TempProject::new();
    std::os::unix::fs::symlink(project.path("nowhere"), project.path("dangling")).unwrap();
    assert_eq!(
        user_error(project.root(), json!([{ "path": "dangling" }])),
        "project root marker path does not exist: dangling"
    );
}

#[cfg(unix)]
#[test]
fn a_raw_lstat_error_in_a_marker_path_exits_2() {
    let project = TempProject::new();
    project.file("afile", "x");
    let err = check(project.root(), json!([{ "path": "afile/sub/x" }])).unwrap_err();
    assert_eq!(
        err,
        Error::Internal(format!(
            "ENOTDIR: not a directory, lstat '{}'",
            project.path("afile/sub").display()
        ))
    );
    assert_eq!(err.exit_code(), 2);
}

// --- packageJsonName markers -----------------------------------------------

#[test]
fn package_json_name_markers() {
    let project = TempProject::new();
    let root = project.root();
    assert_eq!(
        user_error(root, json!([{ "packageJsonName": "demo-suite" }])),
        "project root marker packageJsonName requires package.json in the current directory"
    );

    project.package_json("demo-suite");
    assert_eq!(
        check(root, json!([{ "packageJsonName": "demo-suite" }])),
        Ok(())
    );
    assert_eq!(
        user_error(root, json!([{ "packageJsonName": "other" }])),
        "current directory package.json name does not match other"
    );
    // Strict comparison: no trimming, case-sensitive.
    assert_eq!(
        user_error(root, json!([{ "packageJsonName": " demo-suite" }])),
        "current directory package.json name does not match  demo-suite"
    );
    assert_eq!(
        user_error(root, json!([{ "packageJsonName": "Demo-Suite" }])),
        "current directory package.json name does not match Demo-Suite"
    );
}

#[test]
fn package_json_contents_that_do_not_match() {
    let project = TempProject::new();
    let root = project.root();
    for (contents, name) in [
        ("[1]", "demo-suite"),
        ("{\"name\": 5}", "5"),
        ("null", "x"),
        ("\"demo-suite\"", "demo-suite"),
        ("{}", "x"),
    ] {
        project.file("package.json", contents);
        assert_eq!(
            user_error(root, json!([{ "packageJsonName": name }])),
            format!("current directory package.json name does not match {name}"),
            "{contents}"
        );
    }
}

#[test]
fn package_json_read_and_parse_failures() {
    let project = TempProject::new();
    let root = project.root();
    let prefix = "failed to read package.json for project root validation: ";

    project.file("package.json", "{");
    let message = user_error(root, json!([{ "packageJsonName": "demo-suite" }]));
    assert!(
        message.starts_with(prefix) && message.len() > prefix.len(),
        "{message}"
    );

    // A UTF-8 BOM is kept by `readFile` and rejected by the JSON parser.
    project.file("package.json", "\u{feff}{\"name\":\"demo-suite\"}");
    let message = user_error(root, json!([{ "packageJsonName": "demo-suite" }]));
    assert!(
        message.starts_with(prefix) && message.len() > prefix.len(),
        "{message}"
    );

    // Invalid UTF-8 decodes to U+FFFD instead of failing.
    fs::write(project.path("package.json"), b"{\"name\":\"\xff\"}").unwrap();
    assert_eq!(
        check(root, json!([{ "packageJsonName": "\u{fffd}" }])),
        Ok(())
    );

    // A directory named package.json fails on read, not on stat.
    fs::remove_file(project.path("package.json")).unwrap();
    project.dir("package.json");
    assert_eq!(
        user_error(root, json!([{ "packageJsonName": "demo-suite" }])),
        format!("{prefix}EISDIR: illegal operation on a directory, read")
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_package_json_reports_the_open_error() {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new();
    project.package_json("demo-suite");
    let path = project.path("package.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read(&path).is_ok();
    let result = check(project.root(), json!([{ "packageJsonName": "demo-suite" }]));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    if readable {
        // Running as root: permissions are not enforced.
        return;
    }
    assert_eq!(
        result,
        Err(Error::User(format!(
            "failed to read package.json for project root validation: EACCES: permission denied, open '{}'",
            path.display()
        )))
    );
}

// --- ordering --------------------------------------------------------------

#[test]
fn markers_run_in_order_and_path_before_package_json_name() {
    let project = TempProject::new();
    project.dir("services");
    let root = project.root();
    assert_eq!(
        user_error(
            root,
            json!([{ "path": "missing", "packageJsonName": "nope" }])
        ),
        "project root marker path does not exist: missing"
    );
    assert_eq!(
        user_error(
            root,
            json!([{ "path": "services" }, { "path": "gone" }, { "packageJsonName": "x" }])
        ),
        "project root marker path does not exist: gone"
    );
    assert_eq!(
        user_error(
            root,
            json!([{ "path": "services", "packageJsonName": "x" }])
        ),
        "project root marker packageJsonName requires package.json in the current directory"
    );
}

#[test]
fn empty_marker_fields_are_skipped_like_js_falsy_values() {
    // Unreachable through `parse_config`, but the model allows it.
    let project = TempProject::new();
    let config = Config {
        project: Some(ProjectConfig {
            id: None,
            root_markers: Some(vec![RootMarker {
                path: Some(String::new()),
                package_json_name: Some(String::new()),
            }]),
        }),
        clients: None,
        environments: None,
        targets: Vec::new(),
        tags: None,
    };
    assert_eq!(validate_project_root(&config, project.root()), Ok(()));
}
