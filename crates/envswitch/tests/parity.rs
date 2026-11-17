#[path = "parity/mod.rs"]
mod parity;
#[path = "support/process.rs"]
mod process;
mod support;

#[test]
#[ignore = "requires Bun and the pinned TS oracle; run with --ignored"]
fn scenario_parity() {
    parity::run();
}

#[test]
#[ignore = "parity: scaffold transport smoke requires Bun"]
fn scaffold_version_transcript() {
    parity::scaffold_version_smoke();
}
