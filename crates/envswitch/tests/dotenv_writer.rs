use std::fs;

use envswitch::dotenv::{stringify_env, write_env_file};
use envswitch::error::Error;
use envswitch::model::EnvMap;

fn values(pairs: &[(&str, &str)]) -> EnvMap {
    pairs
        .iter()
        .map(|(key, value)| ((*key).into(), (*value).into()))
        .collect()
}

#[test]
fn empty_map_is_one_newline() {
    assert_eq!(stringify_env(&EnvMap::new()).unwrap(), "\n");
}

#[test]
#[ignore = "needs C1"]
fn serialization_matches_all_22_oracle_goldens() {
    for (value, expected) in [
        ("", "K=\n"),
        ("plain", "K=plain\n"),
        ("two words", "K=\"two words\"\n"),
        ("a=b", "K=a=b\n"),
        ("q\"x", "K=\"q\\\"x\"\n"),
        ("s'x", "K=\"s'x\"\n"),
        ("b`x", "K=\"b`x\"\n"),
        ("$HOME", "K=\"$HOME\"\n"),
        ("back\\slash", "K=\"back\\\\slash\"\n"),
        ("hash#", "K=\"hash#\"\n"),
        ("tab\t", "K=\"tab\\t\"\n"),
        ("nl\nx", "K=\"nl\\nx\"\n"),
        ("cr\rx", "K=\"cr\\rx\"\n"),
        ("\u{1}ctl", "K=\u{1}ctl\n"),
        ("nbsp\u{a0}", "K=\"nbsp\u{a0}\"\n"),
        ("nel\u{85}", "K=nel\u{85}\n"),
        ("bom\u{feff}", "K=\"bom\u{feff}\"\n"),
        ("ls\u{2028}", "K=\"ls\u{2028}\"\n"),
        ("é", "K=é\n"),
        ("\u{7f}", "K=\u{7f}\n"),
        ("x\u{b}", "K=\"x\\u000b\"\n"),
        ("\0", "K=\0\n"),
    ] {
        assert_eq!(
            stringify_env(&values(&[("K", value)])).unwrap(),
            expected,
            "value: {value:?}"
        );
    }
}

#[test]
#[ignore = "needs C1"]
fn sorts_keys_and_reports_first_invalid_key_in_utf16_order() {
    assert_eq!(
        stringify_env(&values(&[("B", "two words"), ("A", "1"), ("EMPTY", "")])).unwrap(),
        "A=1\nB=\"two words\"\nEMPTY=\n"
    );
    for (pairs, first) in [
        (vec![("B-1", ""), ("A-1", "")], "A-1"),
        (vec![("\u{e000}", ""), ("\u{10000}", "")], "\u{10000}"),
        (vec![("A\n", "")], "A\n"),
    ] {
        assert_eq!(
            stringify_env(&values(&pairs)).unwrap_err(),
            Error::internal(format!("invalid environment variable key: {first}"))
        );
    }
}

#[test]
#[ignore = "needs C1"]
fn writes_env_file_and_replaces_existing_content_without_temp_leftovers() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("nested/.env");
    write_env_file(
        &output,
        &values(&[("TOKEN", "secret"), ("NAME", "demo app")]),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "NAME=\"demo app\"\nTOKEN=secret\n"
    );
    write_env_file(&output, &values(&[("NEW", "value")])).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), "NEW=value\n");
    assert_eq!(fs::read_dir(output.parent().unwrap()).unwrap().count(), 1);
}

#[test]
#[ignore = "needs C1"]
fn serialization_failure_preserves_existing_file_and_creates_no_directory() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join(".env");
    fs::write(&output, "EXISTING=1\n").unwrap();
    let invalid = values(&[("BAD-KEY", "nope")]);
    assert_eq!(
        write_env_file(&output, &invalid).unwrap_err(),
        Error::internal("invalid environment variable key: BAD-KEY")
    );
    assert_eq!(fs::read_to_string(&output).unwrap(), "EXISTING=1\n");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    let nested = root.path().join("not-created/.env");
    assert!(write_env_file(&nested, &invalid).is_err());
    assert!(!nested.parent().unwrap().exists());
}

#[test]
fn rename_failure_cleans_up_temp_and_uses_node_error_format() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join(".env");
    fs::create_dir(&output).unwrap();
    let error = write_env_file(&output, &EnvMap::new()).unwrap_err();
    assert_eq!(error.exit_code(), 2);
    let message = error.to_string();
    #[cfg(unix)]
    assert!(
        message.starts_with("EISDIR: illegal operation on a directory, rename '"),
        "{message}"
    );
    assert!(
        message.contains(&format!("' -> '{}'", output.display())),
        "{message}"
    );
    assert!(
        message.contains(&format!("..env.{}.", std::process::id())),
        "{message}"
    );
    let temp = message.split('\'').nth(1).unwrap();
    let uuid = temp.rsplit('.').nth(1).unwrap();
    assert_eq!(uuid.len(), 36);
    assert!(output.is_dir());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn mkdir_failure_reports_node_syscall_without_leaving_files() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("file");
    fs::write(&parent, "original").unwrap();
    let error = write_env_file(&parent.join("nested/.env"), &EnvMap::new()).unwrap_err();
    let message = error.to_string();
    #[cfg(unix)]
    assert!(
        message.starts_with("ENOTDIR: not a directory, mkdir '"),
        "{message}"
    );
    assert!(message.contains(", mkdir '"), "{message}");
    assert_eq!(fs::read_to_string(parent).unwrap(), "original");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
#[cfg(unix)]
fn unix_modes_are_private_and_existing_directory_mode_is_unchanged() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o711)).unwrap();
    let output = root.path().join("nested/deeper/.env");
    write_env_file(&output, &EnvMap::new()).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), "\n");
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for dir in [
        root.path().join("nested"),
        root.path().join("nested/deeper"),
    ] {
        let mode = fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode & !0o750, 0);
    }
    assert_eq!(
        fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
        0o711
    );
    fs::set_permissions(&output, fs::Permissions::from_mode(0o644)).unwrap();
    write_env_file(&output, &EnvMap::new()).unwrap();
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
#[cfg(unix)]
fn replaces_final_symlink_without_modifying_referent() {
    let root = tempfile::tempdir().unwrap();
    let referent = root.path().join("original");
    fs::write(&referent, "untouched").unwrap();
    let output = root.path().join(".env");
    std::os::unix::fs::symlink(&referent, &output).unwrap();
    write_env_file(&output, &EnvMap::new()).unwrap();
    assert!(
        !fs::symlink_metadata(&output)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "\n");
    assert_eq!(fs::read_to_string(referent).unwrap(), "untouched");
}
