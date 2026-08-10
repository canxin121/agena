use std::io;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use process_control::{ChildExt as _, Control as _};

const MAX_PROCESS_OUTPUT_BYTES_PER_STREAM: usize = 16 * 1024 * 1024;

fn bounded_pipe_filter(truncated: Arc<AtomicBool>) -> impl process_control::PipeFilter {
    let mut retained = 0_usize;
    move |chunk: &[u8]| {
        if retained.saturating_add(chunk.len()) <= MAX_PROCESS_OUTPUT_BYTES_PER_STREAM {
            retained += chunk.len();
            Ok(true)
        } else {
            retained = MAX_PROCESS_OUTPUT_BYTES_PER_STREAM;
            truncated.store(true, Ordering::Release);
            Ok(false)
        }
    }
}

pub(crate) fn command_output(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<process_control::Output> {
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("GPG_TTY", "");
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout_truncated = Arc::new(AtomicBool::new(false));
    let stderr_truncated = Arc::new(AtomicBool::new(false));
    let mut output = child
        .controlled_with_output()
        .stdout_filter(bounded_pipe_filter(Arc::clone(&stdout_truncated)))
        .stderr_filter(bounded_pipe_filter(Arc::clone(&stderr_truncated)))
        .time_limit(timeout)
        .terminate_for_timeout()
        .wait()?
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "process timed out"))?;
    if stdout_truncated.load(Ordering::Acquire) {
        output
            .stdout
            .extend_from_slice(b"\n... process stdout truncated at 16 MiB\n");
    }
    if stderr_truncated.load(Ordering::Acquire) {
        output
            .stderr
            .extend_from_slice(b"\n... process stderr truncated at 16 MiB\n");
    }
    Ok(output)
}

#[cfg(all(test, unix))]
mod tests {
    use super::command_output;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn command_output_terminates_a_timed_out_child() {
        let started = Instant::now();
        let error = command_output(
            Command::new("sh").args(["-c", "sleep 2"]),
            Duration::from_millis(50),
        )
        .expect_err("child must time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
