use std::io;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

const MAX_PROCESS_OUTPUT_BYTES_PER_STREAM: usize = 16 * 1024 * 1024;

pub(crate) fn command_output(mut command: Command, timeout: Duration) -> io::Result<Output> {
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("GPG_TTY", "")
        .stdin(Stdio::null());
    // Partial Git/Rift output cannot be treated as a complete inventory.
    agena_process::blocking::output(command, timeout, MAX_PROCESS_OUTPUT_BYTES_PER_STREAM)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn command_output_terminates_a_timed_out_child() {
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 2"]);
        let error = command_output(command, Duration::from_millis(50)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn leader_exit_cleans_descendants_and_preserves_actual_failure_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sleep 30 & printf '%s' $! > \"$1\"; printf diagnostic >&2; exit 7",
                "fixture",
            ])
            .arg(dir.path().join("pid"));
        let output = command_output(command, Duration::from_secs(5)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stderr, b"diagnostic");
        let pid: i32 = std::fs::read_to_string(dir.path().join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            // SAFETY: signal zero only observes the fixture's positive PID.
            if unsafe { libc::kill(pid, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "background descendant remained alive"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
