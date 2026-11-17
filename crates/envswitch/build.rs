//! Exposes the compilation target triple to the crate as `env!("TARGET")`,
//! used by `--version` (D9): `envswitch <version> <target-triple>`.
//!
//! Also sets `cfg(envswitch_release_target)` when compiling for a triple that
//! the release pipeline ships. Only those triples can appear in an install
//! receipt (`install/receipt.rs`, `scripts/install.*`), so a binary built for
//! any other triple (for example the `x86_64-unknown-linux-gnu` host default on
//! Linux) can never locate a valid receipt and `update` always refuses it. Tests
//! that need an installable binary are ignored, with a reason, on such builds.

/// Keep in sync with the native targets accepted by `install::receipt`.
const RELEASE_TARGETS: [&str; 5] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
];

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=TARGET={target}");
    println!("cargo:rustc-check-cfg=cfg(envswitch_release_target)");
    if RELEASE_TARGETS.contains(&target.as_str()) {
        println!("cargo:rustc-cfg=envswitch_release_target");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
