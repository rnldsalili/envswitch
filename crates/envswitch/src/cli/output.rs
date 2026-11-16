//! Progress formatting and summaries (contract 01 §4). Owner: C4.

// Stub file: remove when implemented.
#![allow(unused_variables, clippy::todo)]

use crate::model::ProgressEvent;

/// TS `formatProgressEvent(event, { color })` (01 §4.2), no trailing newline.
/// Plain: `envswitch [<status>] <message>`. Color: hard-coded SGR bytes
/// (`\x1b[2m`, `\x1b[32m`/`\x1b[33m`/`\x1b[34m`, `\x1b[0m`).
pub fn format_progress_event(event: &ProgressEvent, color: bool) -> String {
    todo!("C4: 01 §4.2")
}
