//! Help text (contract 01 §3.5). Owner: C4.
//! Everything here is `pub(crate)` and C4 may reshape it freely.

/// The exact stdout bytes of every help path, including the trailing blank
/// line (`...config item.\n\n`).
pub(crate) fn help_text() -> &'static str {
    r"envswitch

Usage:
  envswitch
  envswitch switch --vault <vault> --config-item <item> --client <client> [--env local] [--target all|name] [--dry-run]
  envswitch list --vault <vault> --config-item <item> [clients|targets|layers]
  envswitch doctor --vault <vault> --config-item <item>
  envswitch update

Notes:
  Run envswitch with no arguments for guided switch prompts.
  Run envswitch update to reinstall the latest GitHub release in place.
  Run envswitch from the root of the project you want to update.
  Project metadata, target paths, and tag rules are loaded from the 1Password config item.

"
}
