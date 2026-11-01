use std::{
    env,
    ffi::OsStr,
    path::Path,
    process::{Command, ExitStatus, Output, Stdio},
    sync::{Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant},
};

use crate::provider_error::ProviderError;

const DEFAULT_INTERACTIVE_TIMEOUT_SECS: u64 = 300;
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) fn helper_is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn run_interactive(
    command: &mut Command,
    operation: impl Into<String>,
) -> Result<ExitStatus, ProviderError> {
    run_interactive_guarded(command, operation, &mut || Ok(()))
}

pub(crate) fn run_interactive_guarded(
    command: &mut Command,
    operation: impl Into<String>,
    guard: &mut dyn FnMut() -> Result<(), ProviderError>,
) -> Result<ExitStatus, ProviderError> {
    let operation = operation.into();
    let timeout = interactive_timeout();
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let _signal_lock = helper_signal_lock();
    let mut child = command.spawn().map_err(ProviderError::Io)?;
    let _interrupt_guard = interrupt_guard_for_child(&mut child)?;
    wait_for_child(&mut child, timeout, operation.as_str(), guard)
}

pub(crate) fn run_probe<I, S>(program: &Path, args: I) -> Result<Output, ProviderError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A leader can exit while descendants retain stdout/stderr. The shared
    // runner bounds pipe EOF as well as child lifetime and cleans up the tree;
    // joining raw blocking readers after wait_for_child did neither.
    agena_process::blocking::output(command, PROBE_TIMEOUT, 256 * 1024)
        .map_err(|error| helper_io_error(error, "terminal helper probe", PROBE_TIMEOUT))
}

pub(crate) fn run_with_input(
    mut command: Command,
    input: Vec<u8>,
    operation: impl Into<String>,
    timeout: Duration,
) -> Result<ExitStatus, ProviderError> {
    let operation = operation.into();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    agena_process::blocking::output_with_input(command, input, timeout, 256 * 1024)
        .map(|output| output.status)
        .map_err(|error| helper_io_error(error, &operation, timeout))
}

fn helper_io_error(error: std::io::Error, operation: &str, timeout: Duration) -> ProviderError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        ProviderError::Timeout {
            operation: operation.to_owned(),
            seconds: timeout.as_secs().max(1),
        }
    } else {
        ProviderError::Io(error)
    }
}

fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
    operation: &str,
    guard: &mut dyn FnMut() -> Result<(), ProviderError>,
) -> Result<ExitStatus, ProviderError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(ProviderError::Io)? {
            return Ok(status);
        }
        if let Err(error) = guard() {
            stop_child_after_failure(child, operation);
            return Err(error);
        }
        if Instant::now() >= deadline {
            stop_child_after_failure(child, operation);
            return Err(ProviderError::Timeout {
                operation: operation.to_owned(),
                seconds: timeout.as_secs().max(1),
            });
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn stop_child_after_failure(child: &mut std::process::Child, operation: &str) {
    if let Err(error) = child.kill()
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        tracing::warn!(
            operation,
            diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
            "failed to terminate a TUI helper after its operation failed"
        );
    }
    if let Err(error) = child.wait() {
        tracing::warn!(
            operation,
            diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
            "failed to reap a TUI helper after its operation failed"
        );
    }
}

fn interactive_timeout() -> Duration {
    let seconds = env::var("AGENA_TUI_HELPER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| (15..=3_600).contains(seconds))
        .unwrap_or(DEFAULT_INTERACTIVE_TIMEOUT_SECS);
    Duration::from_secs(seconds)
}

pub(crate) fn interrupted_status(status: ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        matches!(status.signal(), Some(libc::SIGINT) | Some(libc::SIGTERM))
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        false
    }
}

fn helper_signal_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct ParentInterruptGuard {
    #[cfg(unix)]
    previous: libc::sighandler_t,
}

fn interrupt_guard_for_child(
    child: &mut std::process::Child,
) -> Result<ParentInterruptGuard, ProviderError> {
    match ParentInterruptGuard::install() {
        Ok(guard) => Ok(guard),
        Err(error) => {
            stop_child_after_failure(child, "install parent interrupt guard");
            Err(error)
        }
    }
}

impl ParentInterruptGuard {
    fn install() -> Result<Self, ProviderError> {
        #[cfg(unix)]
        {
            // Install only after spawning: ignored dispositions survive exec,
            // while the helper must retain the default SIGINT behavior so the
            // user's Ctrl+C cancels it without terminating Agena.
            // SAFETY: signal is called with a valid signal number and handler.
            let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
            if previous == libc::SIG_ERR {
                return Err(ProviderError::Io(std::io::Error::last_os_error()));
            }
            Ok(Self { previous })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }
}

impl Drop for ParentInterruptGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: previous was returned by signal for SIGINT in install.
            let _ = unsafe { libc::signal(libc::SIGINT, self.previous) };
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn probe_does_not_wait_for_descendants_holding_output_pipes() {
        let start = Instant::now();
        let output = run_probe(Path::new("/bin/sh"), ["-c", "sleep 5 & printf ready"])
            .expect("leader output must be collected without waiting for its descendant");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ready");
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn stdin_delivery_shares_the_process_deadline() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 5"]);
        let start = Instant::now();
        let result = run_with_input(
            command,
            vec![b'x'; 4 * 1024 * 1024],
            "blocked stdin",
            Duration::from_millis(100),
        );
        assert!(
            matches!(result, Err(ProviderError::Timeout { .. })),
            "{result:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn exiting_leader_cannot_leave_a_blocked_stdin_writer() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 5 & exit 0"]);
        let start = Instant::now();
        let result = run_with_input(
            command,
            vec![b'x'; 4 * 1024 * 1024],
            "orphaned stdin",
            Duration::from_millis(500),
        );
        assert!(result.is_err(), "not all input could have been delivered");
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn noisy_probe_is_bounded_without_blocking_on_a_full_pipe() {
        let start = Instant::now();
        let result = run_probe(
            Path::new("/bin/sh"),
            [
                "-c",
                "head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2",
            ],
        );
        assert!(
            matches!(result, Err(ProviderError::Io(ref error))
            if error.kind() == std::io::ErrorKind::FileTooLarge),
            "{result:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn successful_input_is_fully_written() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "test \"$(wc -c)\" -eq 1048576"]);
        assert!(
            run_with_input(
                command,
                vec![b'x'; 1024 * 1024],
                "complete stdin",
                Duration::from_secs(2),
            )
            .unwrap()
            .success()
        );
    }
}
