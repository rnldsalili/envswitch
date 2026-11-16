#[path = "parity/mod.rs"]
mod parity;
mod support;

#[test]
#[ignore = "parity: enable at integration"]
fn scenario_parity() {
    parity::run();
}

#[test]
#[ignore = "parity: scaffold transport smoke requires Bun"]
fn scaffold_version_transcript() {
    parity::scaffold_version_smoke();
}
