use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use command_group::CommandGroup;

use crate::sandbox::{SandboxError, SandboxExecOutput};

pub(super) fn configure_piped_stdio(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
}

pub(super) fn run_command_with_timeout(
    command: &mut Command,
    timeout_ms: Option<u64>,
    start: Instant,
) -> Result<SandboxExecOutput, SandboxError> {
    let mut child = command.group_spawn()?;

    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout pipe unavailable"))?;
    let stderr = child
        .inner()
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("child stderr pipe unavailable"))?;

    let stdout_thread = thread::spawn(move || read_pipe(stdout));
    let stderr_thread = thread::spawn(move || read_pipe(stderr));

    let mut timed_out = false;
    if let Some(timeout_ms) = timeout_ms {
        let timeout = Duration::from_millis(timeout_ms);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }

            if start.elapsed() >= timeout {
                timed_out = true;
                let _ = child.kill();
                break;
            }

            thread::sleep(Duration::from_millis(10));
        }
    }

    let status = child.wait()?;
    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = String::from_utf8_lossy(&stderr).to_string();

    Ok(SandboxExecOutput {
        exit_code: if timed_out {
            124
        } else {
            status.code().unwrap_or(-1)
        },
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        aggregated_output: format!("{stdout}{stderr}"),
        duration: start.elapsed(),
        timed_out,
    })
}

fn read_pipe(mut pipe: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = pipe.read_to_end(&mut output);
    output
}
