use std::io::{Read, Write};
use std::sync::Arc;

use envswitch::error::{Error, Result};
use envswitch::op::OnePasswordClient;
use envswitch::op::runner::{ExecCommandRunner, MAX_STDOUT_BYTES};
use envswitch::ports::{CommandOutput, CommandRunner, OnePassword};

// Re-executed by ExecCommandRunner; no shell or external executable is needed.
#[test]
fn process_child() {
    let args: Vec<_> = std::env::args().collect();
    let Some(marker) = args.iter().find(|arg| arg.starts_with("envswitch-child-")) else {
        return;
    };
    assert!(
        args.iter()
            .any(|arg| arg == "literal space ; $HOME `no-shell`")
    );
    match marker.as_str() {
        #[cfg(unix)]
        "envswitch-child-stdin" => {
            use std::os::fd::AsFd;
            use std::os::unix::fs::FileTypeExt;
            let mut stdin =
                std::fs::File::from(std::io::stdin().as_fd().try_clone_to_owned().unwrap());
            assert!(!stdin.metadata().unwrap().file_type().is_char_device());
            assert_eq!(stdin.read(&mut [0]).unwrap(), 0);
            std::process::exit(0);
        }
        "envswitch-child-failure" => {
            std::io::stdout().write_all(b"SUPER_SECRET").unwrap();
            std::io::stderr().write_all(b"  safe failure  \n").unwrap();
            std::process::exit(1);
        }
        "envswitch-child-cap" | "envswitch-child-cap-stderr" => {
            if marker.ends_with("-stderr") {
                std::io::stderr().write_all(b"  safe overflow  \n").unwrap();
            }
            let chunk = [b'x'; 8192];
            let mut stdout = std::io::stdout().lock();
            for _ in 0..=MAX_STDOUT_BYTES / chunk.len() {
                if stdout.write_all(&chunk).is_err() {
                    break;
                }
            }
            std::process::exit(0);
        }
        "envswitch-child-stderr-cap" => {
            let chunk = [b' '; 8192];
            let mut stderr = std::io::stderr().lock();
            for _ in 0..=MAX_STDOUT_BYTES / chunk.len() {
                if stderr.write_all(&chunk).is_err() {
                    break;
                }
            }
            std::process::exit(0);
        }
        "envswitch-child-success" => {
            std::io::stdout().write_all(b"raw\0bytes\xff").unwrap();
            std::io::stderr().write_all(b"ignored on success").unwrap();
            std::process::exit(0);
        }
        _ => {
            let count: usize = marker
                .strip_prefix("envswitch-child-bytes-")
                .unwrap()
                .parse()
                .unwrap();
            std::io::copy(
                &mut std::io::repeat(b'x').take(count as u64),
                &mut std::io::stdout(),
            )
            .unwrap();
            std::process::exit(0);
        }
    }
}

fn child(marker: &str) -> Result<CommandOutput> {
    ExecCommandRunner.run(
        std::env::current_exe().unwrap().to_str().unwrap(),
        &[
            "--exact".into(),
            "process_child".into(),
            "--nocapture".into(),
            "--skip".into(),
            marker.into(),
            "--skip".into(),
            "literal space ; $HOME `no-shell`".into(),
        ],
    )
}

#[test]
fn captures_nonzero_output_for_client_mapping_on_unix_and_windows() {
    let output = child("envswitch-child-failure").unwrap();
    assert_eq!(output.code, Some(1));
    assert!(
        output
            .stdout
            .windows(12)
            .any(|bytes| bytes == b"SUPER_SECRET")
    );
    assert_eq!(output.stderr, b"  safe failure  \n");
}

struct ChildRunner;
impl CommandRunner for ChildRunner {
    fn run(&self, _program: &str, _args: &[String]) -> Result<CommandOutput> {
        child("envswitch-child-failure")
    }
}

#[test]
fn real_child_stdout_never_appears_in_error_messages() {
    let op = OnePasswordClient::new("synthetic", Arc::new(ChildRunner));
    let error = op.assert_authenticated().unwrap_err();
    assert_eq!(error, Error::internal("safe failure"));
    assert!(!error.to_string().contains("SUPER_SECRET"));
}

#[test]
fn stdout_over_cap_is_an_internal_error_without_stdout() {
    assert_eq!(
        child("envswitch-child-cap").unwrap_err(),
        Error::internal("stdout maxBuffer length exceeded")
    );
}

#[test]
fn accepts_exact_stdout_cap_and_rejects_one_more_byte() {
    let header_bytes =
        child("envswitch-child-success").unwrap().stdout.len() - b"raw\0bytes\xff".len();
    let count = MAX_STDOUT_BYTES - header_bytes;
    let output = child(&format!("envswitch-child-bytes-{count}")).unwrap();
    assert!(output.success());
    assert_eq!(output.stdout.len(), MAX_STDOUT_BYTES);
    assert_eq!(
        child(&format!("envswitch-child-bytes-{}", count + 1)).unwrap_err(),
        Error::internal("stdout maxBuffer length exceeded")
    );
}

#[test]
fn overflow_prefers_trimmed_stderr_and_caps_both_pipes_like_execfile() {
    assert_eq!(
        child("envswitch-child-cap-stderr").unwrap_err(),
        Error::internal("safe overflow")
    );
    assert_eq!(
        child("envswitch-child-stderr-cap").unwrap_err(),
        Error::internal("stderr maxBuffer length exceeded")
    );
}

#[test]
fn successful_runner_preserves_raw_bytes_and_stderr() {
    let output = child("envswitch-child-success").unwrap();
    assert!(output.success());
    assert!(output.stdout.ends_with(b"raw\0bytes\xff"));
    assert_eq!(output.stderr, b"ignored on success");
}

#[test]
fn spawn_failure_uses_approved_message_format() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("envswitch-no-such-executable");
    let program = missing.to_str().unwrap();
    let error = ExecCommandRunner.run(program, &[]).unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(
        error
            .to_string()
            .starts_with(&format!("failed to run {program}: "))
    );
}

#[test]
#[cfg(unix)]
fn stdin_is_a_non_char_device_pipe_at_eof() {
    let output = child("envswitch-child-stdin").unwrap();
    assert!(
        output.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
