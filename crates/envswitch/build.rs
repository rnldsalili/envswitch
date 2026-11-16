//! Exposes the compilation target triple to the crate as `env!("TARGET")`,
//! used by `--version` (D9): `envswitch <version> <target-triple>`.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
