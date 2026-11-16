//! Binary entry point. Never `fn main() -> Result` (that would print
//! `Error: ...` and exit 1). All behavior lives in `envswitch::cli::run`.

use std::io::Write;

fn main() {
    envswitch::cli::init_process();

    let mut deps = match envswitch::Deps::real() {
        Ok(deps) => deps,
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "envswitch: {err}");
            std::process::exit(err.exit_code());
        }
    };

    // Lossy conversion instead of `std::env::args()`, which panics on
    // non-UTF-8 arguments.
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let code = envswitch::cli::run(argv, &mut deps);
    let _ = deps.out.flush();
    let _ = deps.err.flush();
    drop(deps);
    std::process::exit(code);
}
