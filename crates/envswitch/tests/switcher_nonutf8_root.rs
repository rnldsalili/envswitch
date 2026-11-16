//! DIVERGENCES j1: a project root whose name is not valid UTF-8 (Linux only;
//! macOS APFS and Windows reject such names). Bun decodes the cwd lossily and
//! writes into a sibling directory named with U+FFFD; Rust writes into the
//! actual root the user ran from. Owner: orchestrator (R1 finding).

#![cfg(target_os = "linux")]

mod support;

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;

use envswitch::config::parse_config;
use envswitch::model::{ProgressEvent, SwitchRequest};
use envswitch::switcher::switch_environment;

use support::FakeOp;

#[test]
fn writes_into_the_real_non_utf8_root_not_a_lossy_sibling() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join(OsStr::from_bytes(b"proj\xff"));
    let lossy_sibling = base.path().join("proj\u{FFFD}");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&lossy_sibling).unwrap();

    let config = parse_config(r#"{"targets":[{"name":"api","output":".env"}]}"#).unwrap();
    let request = SwitchRequest {
        config,
        root: root.clone(),
        cwd: root.clone(),
        env: "local".to_string(),
        client: "alpha".to_string(),
        target_name: "api".to_string(),
        dry_run: false,
    };
    let op = FakeOp::new("Demo");
    switch_environment(&op, &request, &mut |_: ProgressEvent| {}).unwrap();

    assert_eq!(fs::read_to_string(root.join(".env")).unwrap(), "\n");
    assert!(
        !lossy_sibling.join(".env").exists(),
        "must not write into the U+FFFD sibling (TS behavior, DIVERGENCES j1)"
    );
}
