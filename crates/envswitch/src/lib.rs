//! envswitch: write local `.env` files from tagged 1Password items.
//!
//! Module index frozen by S1 (PLAN §2.1). Workers never edit this file or any
//! `mod` declaration; they fill in function bodies inside their own files.
//!
//! | Module     | Owner | Contents |
//! |------------|-------|----------|
//! | `error`    | S1    | [`Error`], [`Result`], exit codes, Node-style io messages |
//! | `model`    | S1    | shared data types |
//! | `ports`    | S1    | traits and [`Deps`] |
//! | `jsutil`   | C1    | JS-compatible trim, whitespace, identifier and env-key checks, JSON parse helper |
//! | `config`   | C1    | config parsing, selection, paths, root markers, tag templates |
//! | `op`       | C2    | `op` runner, client, output parsing |
//! | `dotenv`   | C2    | `.env` serialization and atomic write |
//! | `switcher` | C3    | classification, merge, `switch_environment`, `inspect_project` |
//! | `cli`      | C4 (`update.rs`: D3) | argument parsing, help, output, guided mode, commands |
//! | `install`  | D3    | embedded installer, receipt |
//!
//! The library never writes to the process stdout/stderr; only `cli` writes,
//! and only through `Deps::out` / `Deps::err`.

pub mod cli;
pub mod config;
pub mod dotenv;
pub mod error;
pub mod install;
pub mod jsutil;
pub mod model;
pub mod op;
pub mod ports;
pub mod switcher;

pub use cli::run;
pub use error::{Error, Result};
pub use ports::Deps;
