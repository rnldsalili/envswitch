//! JS-compatible helpers (contract 02 §1.4). The whitespace set was probed
//! under Bun 1.4.0: `/\s/.test(c)` and `c.trim() === ""` agree on exactly
//! these code points over every Unicode scalar value.

use std::path::Path;

use envswitch::Error;
use envswitch::jsutil::{
    is_env_key, is_identifier, js_is_space, js_trim, node_basename, parse_json,
};
use serde_json::json;

const JS_WHITESPACE: [u32; 25] = [
    0x9, 0xa, 0xb, 0xc, 0xd, 0x20, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005,
    0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff,
];

#[test]
fn js_is_space_matches_the_probed_set_exactly() {
    for c in (0..=0x10ffff).filter_map(char::from_u32) {
        assert_eq!(
            js_is_space(c),
            JS_WHITESPACE.contains(&(c as u32)),
            "U+{:04X}",
            c as u32
        );
    }
}

#[test]
fn js_trim_differs_from_str_trim_on_bom_and_nel() {
    assert_eq!(js_trim(" \t\u{feff}x y\u{3000}\r\n"), "x y");
    assert_eq!(js_trim("\u{feff}"), "");
    // U+0085 is Unicode White_Space but not JS whitespace.
    assert_eq!(js_trim("\u{85}"), "\u{85}");
    assert_eq!(js_trim(" \u{85}x\u{85} "), "\u{85}x\u{85}");
    assert_eq!("\u{85}".trim(), "");
    assert_eq!("\u{feff}".trim(), "\u{feff}");
    assert_eq!(js_trim(""), "");
    assert_eq!(js_trim("\u{200b}"), "\u{200b}");
}

#[test]
fn identifiers_follow_the_ts_regex() {
    for valid in [
        "a",
        "A9",
        "a-b_c",
        "0",
        "9abc",
        "api",
        "demo-suite",
        "x_1",
        "a--",
        "Z_",
    ] {
        assert!(is_identifier(valid), "{valid:?}");
    }
    for invalid in [
        "",
        "-a",
        "_a",
        "a b",
        "a\n",
        "\na",
        "a\r",
        "\u{e9}",
        "a.b",
        "a/b",
        "a:b",
        "a\u{0}",
        "all apis",
        " a",
        "a\u{feff}",
    ] {
        assert!(!is_identifier(invalid), "{invalid:?}");
    }
}

#[test]
fn env_keys_follow_the_ts_regex() {
    for valid in ["A", "_", "_A1", "a_b", "VALID_KEY", "__proto__", "Z9"] {
        assert!(is_env_key(valid), "{valid:?}");
    }
    for invalid in [
        "",
        "9A",
        "A-B",
        "A B",
        "\u{e9}",
        "A\n",
        "invalid-key",
        "A.B",
        "A\u{0}",
    ] {
        assert!(!is_env_key(invalid), "{invalid:?}");
    }
}

#[test]
fn parse_json_accepts_surrounding_json_whitespace() {
    assert_eq!(
        parse_json(" \t\r\n{\"a\": [1, null]}\n ", "x").unwrap(),
        json!({ "a": [1, null] })
    );
    assert_eq!(parse_json("[]", "x").unwrap(), json!([]));
    assert_eq!(parse_json("\"s\"", "x").unwrap(), json!("s"));
    // Duplicate keys: the last one wins, like `JSON.parse`.
    assert_eq!(
        parse_json(r#"{"a":1,"a":2}"#, "x").unwrap(),
        json!({ "a": 2 })
    );
}

#[test]
fn parse_json_errors_are_user_errors_with_the_source_prefix() {
    for raw in [
        "",
        "{",
        "not-json",
        "[1,]",
        "{} x",
        "\u{feff}{}",
        "\u{a0}{}",
    ] {
        let err = parse_json(raw, "op item list output").unwrap_err();
        assert_eq!(err.exit_code(), 1, "{raw:?}");
        let Error::User(message) = err else {
            panic!("expected a user error for {raw:?}");
        };
        let suffix = message
            .strip_prefix("op item list output is not valid JSON: ")
            .unwrap_or_else(|| panic!("{raw:?}: {message}"));
        assert!(!suffix.is_empty(), "{raw:?}");
    }
}

#[test]
fn node_basename_matches_node() {
    assert_eq!(node_basename(Path::new("/")), "");
    assert_eq!(node_basename(Path::new("")), "");
    assert_eq!(
        node_basename(Path::new("/work/envswitch-project")),
        "envswitch-project"
    );
    assert_eq!(
        node_basename(Path::new("/work/envswitch-project/")),
        "envswitch-project"
    );
    assert_eq!(node_basename(Path::new("/work/x/..")), "..");
    assert_eq!(node_basename(Path::new("relative")), "relative");
    assert_eq!(node_basename(Path::new("/caf\u{e9}")), "caf\u{e9}");
}

#[cfg(unix)]
#[test]
fn node_basename_keeps_backslashes_on_posix() {
    assert_eq!(node_basename(Path::new("/work/a\\b")), "a\\b");
    assert_eq!(node_basename(Path::new("C:\\")), "C:\\");
}

#[cfg(windows)]
#[test]
fn node_basename_uses_win32_rules_on_windows() {
    assert_eq!(node_basename(Path::new("C:\\")), "");
    assert_eq!(node_basename(Path::new("C:")), "");
    assert_eq!(node_basename(Path::new("C:\\work\\proj")), "proj");
    assert_eq!(node_basename(Path::new("C:\\work\\proj\\")), "proj");
    assert_eq!(node_basename(Path::new("C:/work/proj")), "proj");
    assert_eq!(node_basename(Path::new("\\\\srv\\share\\")), "share");
    assert_eq!(node_basename(Path::new("\\\\srv\\share\\proj")), "proj");
}
