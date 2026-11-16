//! JS-compatible string helpers shared by several modules (contract 02 §1.4).
//! Owner: C1. Signatures frozen by S1.

use std::path::Path;

use crate::error::{Error, Result};

/// JS `Array.prototype.sort()` default order for strings: UTF-16 code units.
pub(crate) fn compare_utf16(left: &str, right: &str) -> std::cmp::Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

/// JS `\s` / `String.prototype.trim` whitespace set (02 §1.4):
/// `\t \n \v \f \r`, space, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029,
/// U+202F, U+205F, U+3000, U+FEFF. **Not** U+0085.
///
/// Probed under Bun 1.4.0: `/\s/` and `trim()` agree on exactly this set
/// over every Unicode scalar value.
pub fn js_is_space(c: char) -> bool {
    const SINGLES: [char; 14] = [
        '\t', '\n', '\u{0B}', '\u{0C}', '\r', ' ', '\u{A0}', '\u{1680}', '\u{2028}', '\u{2029}',
        '\u{202F}', '\u{205F}', '\u{3000}', '\u{FEFF}',
    ];
    ('\u{2000}'..='\u{200A}').contains(&c) || SINGLES.contains(&c)
}

/// JS `String.prototype.trim` using [`js_is_space`].
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(js_is_space)
}

/// `^[A-Za-z0-9][A-Za-z0-9_-]*$` (ASCII; `$` is end of input, so a trailing
/// `\n` fails). Config identifiers: project id, target, group, env, client.
pub fn is_identifier(s: &str) -> bool {
    match s.as_bytes().split_first() {
        Some((first, rest)) => {
            first.is_ascii_alphanumeric()
                && rest
                    .iter()
                    .all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        }
        None => false,
    }
}

/// `^[A-Za-z_][A-Za-z0-9_]*$`: valid env var key (`op` field labels in C2,
/// `stringify_env` in C2).
pub fn is_env_key(s: &str) -> bool {
    match s.as_bytes().split_first() {
        Some((first, rest)) => {
            (first.is_ascii_alphabetic() || *first == b'_')
                && rest
                    .iter()
                    .all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }
        None => false,
    }
}

/// TS `parseJson(raw, source)` (`validation.ts`): parse the whole string;
/// on failure `Error::User("<source> is not valid JSON: <serde_json error>")`
/// (suffix differs from Bun, DIVERGENCES a5). Leading/trailing JSON
/// whitespace is fine.
pub fn parse_json(raw: &str, source: &str) -> Result<serde_json::Value> {
    serde_json::from_str(raw)
        .map_err(|err| Error::user(format!("{source} is not valid JSON: {err}")))
}

/// Node `path.basename(p)` for absolute paths: last component, `""` for a
/// filesystem root. Used for the `{project}` fallback (C1) and the switch
/// summary `from <basename(root)>` (C4).
///
/// Uses the host flavor (`path.win32.basename` on Windows, so `C:\` and
/// `\\srv\share\` give `""` and `share`). Trailing separators are ignored
/// and `..` is returned literally, exactly like Node.
pub fn node_basename(path: &Path) -> String {
    crate::config::paths::native_basename(path)
}
