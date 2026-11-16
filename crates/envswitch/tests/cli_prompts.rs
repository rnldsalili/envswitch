#![cfg(unix)]

use std::process::Command;
use std::time::Duration;

use envswitch::cli::guided::InquirePrompter;
use envswitch::ports::{Choice, InputSpec, Prompter, SelectSpec};
use expectrl::{Eof, Expect, Session};

#[test]
fn real_inquire_preserves_defaults_raw_text_filtering_and_cancel() {
    const PROBE_ENV: &str = "ENVSWITCH_C4_PROMPT_PROBE";
    if let Ok(mode) = std::env::var(PROBE_ENV) {
        if mode.starts_with("input-") {
            let answer = InquirePrompter.input(&InputSpec {
                message: "C4 input".into(),
                default: if mode == "input-required" {
                    None
                } else {
                    Some("local".into())
                },
                required: true,
            });
            println!("C4-ANSWER:{answer:?}");
        } else {
            let answer = InquirePrompter.select(&SelectSpec {
                message: "C4 select".into(),
                choices: ["Dry run", "Write .env files"]
                    .into_iter()
                    .map(|label| Choice {
                        label: label.into(),
                    })
                    .collect(),
                default_index: Some(1),
            });
            println!("C4-ANSWER:{answer:?}");
        }
        return;
    }
    for (mode, keys, expected) in [
        ("esc", "\x1b", "Err(Cancelled)"),
        ("ctrl-c", "\x03", "Err(Cancelled)"),
        ("select-default", "\r", "Ok(1)"),
        ("filter", "files\r", "Ok(1)"),
        ("wrap", "\x1b[B\r", "Ok(0)"),
        ("input-text", "  alpha  \r", "Ok(\"  alpha  \")"),
        ("input-default", "\r", "Ok(\"local\")"),
        ("input-required", "\ralpha\r", "Ok(\"alpha\")"),
        ("input-esc", "\x1b", "Err(Cancelled)"),
        ("input-ctrl-c", "\x03", "Err(Cancelled)"),
    ] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "real_inquire_preserves_defaults_raw_text_filtering_and_cancel",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PROBE_ENV, mode)
            .env("TERM", "xterm-256color")
            .env("NO_COLOR", "1");
        let mut session = Session::spawn(command).unwrap();
        session.set_expect_timeout(Some(Duration::from_secs(5)));
        session
            .expect(if mode.starts_with("input-") {
                "C4 input"
            } else {
                "C4 select"
            })
            .unwrap();
        if mode == "input-required" {
            session.send("\r").unwrap();
            session.expect("You must provide a value").unwrap();
            session.send("alpha\r").unwrap();
        } else {
            session.send(keys).unwrap();
        }
        session
            .expect(format!("C4-ANSWER:{expected}"))
            .unwrap_or_else(|error| panic!("{mode}: {error}"));
        session.expect("test result: ok.").unwrap();
        session.expect(Eof).unwrap();
    }
}
