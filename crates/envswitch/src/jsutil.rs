//! JS-compatible string helpers shared by several modules (contract 02 §1.4).
//! Owner: C1. Signatures frozen by S1.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use std::path::Path;

use crate::error::Result;

/// JS `\s` / `String.prototype.trim` whitespace set (02 §1.4):
/// `\t \n \v \f \r`, space, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029,
/// U+202F, U+205F, U+3000, U+FEFF. **Not** U+0085.
pub fn js_is_space(c: char) -> bool {
    todo!("C1: 02 §1.4")
}

/// JS `String.prototype.trim` using [`js_is_space`].
pub fn js_trim(s: &str) -> &str {
    todo!("C1: 02 §1.4")
}

/// `^[A-Za-z0-9][A-Za-z0-9_-]*$` (ASCII; `$` is end of input, so a trailing
/// `\n` fails). Config identifiers: project id, target, group, env, client.
pub fn is_identifier(s: &str) -> bool {
    todo!("C1: 02 §1.2")
}

/// `^[A-Za-z_][A-Za-z0-9_]*$`: valid env var key (`op` field labels in C2,
/// `stringify_env` in C2).
pub fn is_env_key(s: &str) -> bool {
    todo!("C1: 02 §5.4, §6.1")
}

/// TS `parseJson(raw, source)` (`validation.ts`): parse the whole string;
/// on failure `Error::User("<source> is not valid JSON: <serde_json error>")`
/// (suffix differs from Bun, DIVERGENCES a5). Leading/trailing JSON
/// whitespace is fine.
pub fn parse_json(raw: &str, source: &str) -> Result<serde_json::Value> {
    todo!("C1: 02 §1.3, §5.3")
}

/// Node `path.basename(p)` for absolute paths: last component, `""` for a
/// filesystem root. Used for the `{project}` fallback (C1) and the switch
/// summary `from <basename(root)>` (C4).
pub fn node_basename(path: &Path) -> String {
    todo!("C1: 01 §4.3, 02 §3.2")
}
