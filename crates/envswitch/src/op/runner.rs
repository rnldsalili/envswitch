//! Real process runner (contract 02 §5.1). Owner: C2.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use crate::error::{Error, Result};
use crate::jsutil::js_trim;
use crate::ports::{CommandOutput, CommandRunner};

/// TS `execFile` `maxBuffer`: stdout above this fails with
/// `stdout maxBuffer length exceeded`.
pub const MAX_STDOUT_BYTES: usize = 50 * 1024 * 1024;

/// `std::process::Command` runner: no shell, no timeout, inherited env and
/// cwd, stdin not a TTY, stdout capped at [`MAX_STDOUT_BYTES`].
/// Spawn failure → `Error::Internal("failed to run <program>: <io error>")`
/// (DIVERGENCES a6).
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecCommandRunner;

impl CommandRunner for ExecCommandRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| Error::internal(format!("failed to run {program}: {error}")))?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let mut output = CommandOutput::default();
        let mut failure = None;
        thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            let stdout_sender = sender.clone();
            scope.spawn(move || {
                let _ = stdout_sender.send(("stdout", read_pipe(stdout)));
            });
            scope.spawn(move || {
                let _ = sender.send(("stderr", read_pipe(stderr)));
            });
            // Drain both pipes concurrently so a full stderr pipe cannot block stdout.
            for (stream, result) in receiver {
                match result {
                    Ok(mut bytes) => {
                        if bytes.len() > MAX_STDOUT_BYTES && failure.is_none() {
                            failure = Some(format!("{stream} maxBuffer length exceeded"));
                            let _ = child.kill();
                        }
                        bytes.truncate(MAX_STDOUT_BYTES);
                        if stream == "stdout" {
                            output.stdout = bytes;
                        } else {
                            output.stderr = bytes;
                        }
                    }
                    Err(error) => {
                        if failure.is_none() {
                            failure = Some(error.to_string());
                            let _ = child.kill();
                        }
                    }
                }
            }
        });
        let status = child
            .wait()
            .map_err(|error| Error::internal(error.to_string()))?;
        if let Some(failure) = failure {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = if stderr.is_empty() {
                ""
            } else {
                js_trim(&stderr)
            };
            return Err(Error::internal(if stderr.is_empty() {
                failure
            } else {
                stderr.to_owned()
            }));
        }
        output.code = status.code();
        Ok(output)
    }
}

fn read_pipe(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_STDOUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
