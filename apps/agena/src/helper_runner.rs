use std::{
    env,
    ffi::OsStr,
    io::{Read, Write},
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
    let mut child = command.spawn().map_err(ProviderError::Io)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || read_limited(stdout, 256 * 1024));
    let stderr_reader = thread::spawn(move || read_limited(stderr, 256 * 1024));
    let status = wait_for_child(
        &mut child,
        PROBE_TIMEOUT,
        "terminal helper probe",
        &mut || Ok(()),
    )?;
    let stdout = stdout_reader
        .join()
        .unwrap_or_else(|_| Ok(Vec::new()))
        .map_err(ProviderError::Io)?;
    let stderr = stderr_reader
        .join()
        .unwrap_or_else(|_| Ok(Vec::new()))
        .map_err(ProviderError::Io)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub(crate) fn run_with_input(
    command: &mut Command,
    input: Vec<u8>,
    operation: impl Into<String>,
    timeout: Duration,
) -> Result<ExitStatus, ProviderError> {
    let operation = operation.into();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(ProviderError::Io)?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        ProviderError::Protocol(format!("{operation} helper stdin is unavailable"))
    })?;
    let writer = thread::spawn(move || stdin.write_all(input.as_slice()));
    let status = wait_for_child(&mut child, timeout, operation.as_str(), &mut || Ok(()));
    let write_result = writer.join().unwrap_or_else(|_| {
        Err(std::io::Error::other(
            "terminal helper stdin writer panicked",
        ))
    });
    let status = status?;
    write_result.map_err(ProviderError::Io)?;
    Ok(status)
}

fn read_limited<R: Read>(reader: Option<R>, limit: usize) -> std::io::Result<Vec<u8>> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    let mut bytes = Vec::new();
    reader.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
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
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProviderError::Timeout {
                operation: operation.to_owned(),
                seconds: timeout.as_secs().max(1),
            });
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
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
            let _ = child.kill();
            let _ = child.wait();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn probe_captures_output() {
        let output = run_probe(Path::new("/bin/sh"), ["-c", "printf helper-ok"])
            .expect("probe should finish");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"helper-ok");
    }

    #[cfg(unix)]
    #[test]
    fn timed_out_helpers_are_killed_and_reaped() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 10"])
            .spawn()
            .expect("spawn slow helper");
        let error = wait_for_child(
            &mut child,
            Duration::from_millis(30),
            "slow helper",
            &mut || Ok(()),
        )
        .expect_err("helper should time out");
        assert!(matches!(error, ProviderError::Timeout { .. }));
        assert!(child.try_wait().expect("child status").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn failed_transfer_guard_kills_and_reaps_helper() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 10"])
            .spawn()
            .expect("spawn guarded helper");
        let error = wait_for_child(
            &mut child,
            Duration::from_secs(1),
            "guarded helper",
            &mut || {
                Err(ProviderError::Unsupported(
                    "transfer limit reached".to_owned(),
                ))
            },
        )
        .expect_err("guard should stop helper");
        assert!(matches!(error, ProviderError::Unsupported(_)));
        assert!(child.try_wait().expect("child status").is_some());
    }
}
