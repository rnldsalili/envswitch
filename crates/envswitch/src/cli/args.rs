//! Hand-written argument parser (contract 01 §1–2; D4). Owner: C4.

use indexmap::IndexMap;

/// A parsed flag value (TS `string | true`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagValue {
    /// `--flag` with no value (next token missing, `""`, or starting `--`).
    True,
    /// `--flag=value` or `--flag value` (value may be `""` via `=`).
    Value(String),
}

/// TS `ParsedArgs`. Absent flags are absent keys (no defaults applied).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedArgs {
    /// `argv[0]`, whatever it is; `None` only for an empty argv.
    pub command: Option<String>,
    pub positional: Vec<String>,
    /// Insertion order of first occurrence; a repeated flag keeps its first
    /// position and takes the last value (JS object semantics).
    pub flags: IndexMap<String, FlagValue>,
}

/// TS `parseArgs(argv)` (01 §1, including the empty-next-token erratum and
/// DIVERGENCES a3/a4). `argv` excludes the program name.
pub fn parse_args(argv: &[String]) -> ParsedArgs {
    let mut result = ParsedArgs {
        command: argv.first().cloned(),
        ..ParsedArgs::default()
    };
    let mut tokens = argv.iter().skip(1).peekable();
    while let Some(token) = tokens.next() {
        let Some(raw) = token.strip_prefix("--") else {
            result.positional.push(token.clone());
            continue;
        };
        let (name, value) = if let Some((name, value)) = raw.split_once('=') {
            (name, FlagValue::Value(value.to_string()))
        } else if tokens
            .peek()
            .is_some_and(|next| !next.is_empty() && !next.starts_with("--"))
        {
            (raw, FlagValue::Value(tokens.next().unwrap().clone()))
        } else {
            (raw, FlagValue::True)
        };
        result.flags.insert(name.to_string(), value);
    }
    result
}
