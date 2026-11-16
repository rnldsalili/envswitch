//! Lenient parsing of `op` JSON output (contract 02 §5.3). Owner: C2.

use serde_json::Value;

use crate::error::{Error, Result};
use crate::jsutil::parse_json;
use crate::model::{ItemSummary, Vault};

/// One `fields[i]` entry: `label`/`value` strings, else `""`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RawField {
    pub label: String,
    pub value: String,
}

fn string(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").into()
}

/// `op vault list` stdout → vaults (`name || id`).
pub(crate) fn parse_vault_list(raw: &str) -> Result<Vec<Vault>> {
    let parsed = parse_json(raw, "op vault list output")?;
    let list = parsed
        .as_array()
        .ok_or_else(|| Error::user("op vault list returned non-array JSON"))?;
    list.iter()
        .enumerate()
        .map(|(index, value)| {
            if !value.is_object() {
                return Err(Error::user(format!(
                    "op vault list result[{index}] must be an object"
                )));
            }
            let id = string(value, "id");
            let name = string(value, "name");
            if id.is_empty() && name.is_empty() {
                return Err(Error::user(format!(
                    "op vault list result[{index}] must include id or name"
                )));
            }
            Ok(Vault {
                name: if name.is_empty() { id.clone() } else { name },
                id,
            })
        })
        .collect()
}

/// `op item list` stdout → items, API order preserved.
pub(crate) fn parse_item_list(raw: &str) -> Result<Vec<ItemSummary>> {
    let parsed = parse_json(raw, "op item list output")?;
    let list = parsed
        .as_array()
        .ok_or_else(|| Error::user("op item list returned non-array JSON"))?;
    list.iter()
        .enumerate()
        .map(|(index, value)| {
            if !value.is_object() {
                return Err(Error::user(format!(
                    "op item list result[{index}] must be an object"
                )));
            }
            let id = string(value, "id");
            let title = string(value, "title");
            if id.is_empty() && title.is_empty() {
                return Err(Error::user(format!(
                    "op item list result[{index}] must include id or title"
                )));
            }
            let tags = value
                .get("tags")
                .and_then(Value::as_array)
                .map(|tags| {
                    tags.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Ok(ItemSummary { id, title, tags })
        })
        .collect()
}

/// `op item get <item>` stdout → fields, in order.
pub(crate) fn parse_item_fields(raw: &str, item: &str) -> Result<Vec<RawField>> {
    let parsed = parse_json(raw, &format!("op item get {item} output"))?;
    let fields = parsed
        .as_object()
        .and_then(|object| object.get("fields"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::user(format!("op item get {item} returned unexpected JSON")))?;
    fields
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if !value.is_object() {
                return Err(Error::user(format!(
                    "op item field[{index}] must be an object"
                )));
            }
            Ok(RawField {
                label: string(value, "label"),
                value: string(value, "value"),
            })
        })
        .collect()
}
