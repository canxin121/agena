//! In-tree shell executor.
//!
//! `tool::shell` provides a small synchronous command runner with a watchdog
//! timeout, stdout/stderr capture, and shell-injection environment scrubbing.
//! It deliberately does *not* implement OS-level sandboxing. Agena gates
//! filesystem and tool access through `crate::permission` instead.

use std::collections::HashMap;
use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agena_tool::{ShellError, ShellOutput, ShellRequest};
use tokio_util::sync::CancellationToken;

/// Run a command synchronously, scrubbing dangerous loader env vars and
/// enforcing `timeout_ms` via a watchdog thread.
pub fn execute(
    request: &ShellRequest,
    cancellation: Option<&CancellationToken>,
) -> Result<ShellOutput, ShellError> {
    validate(request)?;

    let env = sanitize_env(&request.env);
    let (program, args) = request
        .command
        .split_first()
        .ok_or_else(|| ShellError::InvalidRequest("command must not be empty".to_string()))?;

    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(env.iter())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Put the shell and all of its descendants in a dedicated process group.
    // Killing only `sh -c` can otherwise leave grandchildren running after
    // Ctrl+C, which is especially dangerous for mutating commands.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let started = Instant::now();
    let mut child = command.spawn().map_err(ShellError::Spawn)?;

    // Drain stdout / stderr off-thread so a child that fills its pipe
    // buffers cannot deadlock the parent.
    let stdout_handle = child.stdout.take().map(spawn_drain);
    let stderr_handle = child.stderr.take().map(spawn_drain);

    let timeout = request.timeout_ms.map(Duration::from_millis);
    let wait_outcome = wait_with_timeout(&mut child, timeout, cancellation)?;

    let duration = started.elapsed();
    let stdout = collect_drain(stdout_handle);
    let stderr = collect_drain(stderr_handle);

    if matches!(wait_outcome, WaitOutcome::Cancelled) {
        return Err(ShellError::Cancelled);
    }

    let status = match wait_outcome {
        WaitOutcome::Exited(status) | WaitOutcome::TimedOut(status) => status,
        WaitOutcome::Cancelled => unreachable!("cancelled outcome returned above"),
    };
    let timed_out = matches!(wait_outcome, WaitOutcome::TimedOut(_));
    let exit_code = status_to_code(status);

    let aggregated_output = match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.clone(),
        (true, false) => stderr.clone(),
        (false, false) => format!("{stdout}\n{stderr}"),
    };

    Ok(ShellOutput {
        exit_code,
        stdout,
        stderr,
        aggregated_output,
        duration,
        timed_out,
    })
}

fn validate(request: &ShellRequest) -> Result<(), ShellError> {
    if request.command.is_empty() {
        return Err(ShellError::InvalidRequest(
            "command must contain at least one token".to_string(),
        ));
    }
    if request.command[0].trim().is_empty() {
        return Err(ShellError::InvalidRequest(
            "command executable must not be empty".to_string(),
        ));
    }
    if !request.cwd.exists() {
        return Err(ShellError::InvalidRequest(format!(
            "shell cwd does not exist: {}",
            request.cwd.display()
        )));
    }
    if !request.cwd.is_dir() {
        return Err(ShellError::InvalidRequest(format!(
            "shell cwd is not a directory: {}",
            request.cwd.display()
        )));
    }
    Ok(())
}

/// Strip env vars that can hijack a child shell or dynamic loader.
fn sanitize_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    const BLOCKED_EXACT: &[&str] = &[
        "BASH_ENV",
        "ENV",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
    ];
    const BLOCKED_PREFIXES: &[&str] = &["DYLD_", "LD_", "BASH_FUNC_"];

    env.iter()
        .filter(|(key, _)| {
            !BLOCKED_EXACT
                .iter()
                .any(|name| key.eq_ignore_ascii_case(name))
                && !BLOCKED_PREFIXES
                    .iter()
                    .any(|prefix| starts_with_ascii_case_insensitive(key, prefix))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Option<Duration>,
    cancellation: Option<&CancellationToken>,
) -> Result<WaitOutcome, ShellError> {
    let started = Instant::now();
    let poll_interval = Duration::from_millis(20);
    loop {
        match child.try_wait().map_err(ShellError::Wait)? {
            Some(status) => return Ok(WaitOutcome::Exited(status)),
            None => {
                if cancellation.is_some_and(CancellationToken::is_cancelled) {
                    terminate_process_tree(child)?;
                    return Ok(WaitOutcome::Cancelled);
                }
                if timeout.is_some_and(|deadline| started.elapsed() >= deadline) {
                    let status = terminate_process_tree(child)?;
                    return Ok(WaitOutcome::TimedOut(status));
                }
                thread::sleep(poll_interval);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum WaitOutcome {
    Exited(std::process::ExitStatus),
    TimedOut(std::process::ExitStatus),
    Cancelled,
}

fn terminate_process_tree(
    child: &mut std::process::Child,
) -> Result<std::process::ExitStatus, ShellError> {
    #[cfg(unix)]
    {
        let process_group = -(child.id() as i32);
        // SAFETY: `kill` is called with the freshly spawned child's process
        // group id. Failure is harmless here because the child may have exited
        // between `try_wait` and this signal.
        unsafe {
            libc::kill(process_group, libc::SIGTERM);
        }
        let mut exit_status = None;
        let grace_started = Instant::now();
        while grace_started.elapsed() < Duration::from_millis(150) {
            if exit_status.is_none() {
                exit_status = child.try_wait().map_err(ShellError::Wait)?;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // SAFETY: same process-group reasoning as above. SIGKILL is the
        // bounded fallback used after the cooperative termination grace.
        unsafe {
            libc::kill(process_group, libc::SIGKILL);
        }
        match exit_status {
            Some(status) => Ok(status),
            None => child.wait().map_err(ShellError::Wait),
        }
    }

    #[cfg(windows)]
    {
        // Windows does not expose Unix process groups. `taskkill /T` walks
        // the descendant tree and `/F` provides the same bounded hard-stop
        // semantics as SIGKILL after Ctrl+C.
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .status();
        child.wait().map_err(ShellError::Wait)
    }

    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = child.kill();
        child.wait().map_err(ShellError::Wait)
    }
}

fn status_to_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| {
        // Killed by signal on Unix; surface a stable nonzero code so callers
        // see "failed" rather than "success".
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.signal().map(|s| 128 + s).unwrap_or(-1)
        }
        #[cfg(not(unix))]
        {
            -1
        }
    })
}

fn spawn_drain<R>(mut reader: R) -> thread::JoinHandle<io::Result<String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    })
}

fn collect_drain(handle: Option<thread::JoinHandle<io::Result<String>>>) -> String {
    let Some(handle) = handle else {
        return String::new();
    };
    handle
        .join()
        .ok()
        .and_then(|res| res.ok())
        .unwrap_or_default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn process_exists(pid: i32) -> bool {
        // SAFETY: signal 0 performs existence/permission checking only.
        (unsafe { libc::kill(pid, 0) == 0 })
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    #[test]
    fn cancellation_stops_a_foreground_process_group_promptly() {
        let cancellation = CancellationToken::new();
        let cancel_from_thread = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel_from_thread.cancel();
        });
        let request = ShellRequest {
            command: vec!["sh".to_string(), "-c".to_string(), "sleep 30".to_string()],
            cwd: std::env::current_dir().expect("current directory"),
            env: std::env::vars().collect(),
            timeout_ms: None,
        };
        let started = Instant::now();

        let result = execute(&request, Some(&cancellation));

        canceller.join().expect("canceller thread");
        assert!(matches!(result, Err(ShellError::Cancelled)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancellation took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn cancellation_kills_shell_grandchildren_not_just_the_shell() {
        let pid_path = std::env::temp_dir().join(format!(
            "agena-shell-descendant-{}-{}.pid",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after Unix epoch")
                .as_nanos()
        ));
        let cancellation = CancellationToken::new();
        let cancel_from_thread = cancellation.clone();
        let pid_path_for_thread = pid_path.clone();
        let canceller = thread::spawn(move || {
            let started = Instant::now();
            while !pid_path_for_thread.exists() && started.elapsed() < Duration::from_secs(2) {
                thread::sleep(Duration::from_millis(10));
            }
            cancel_from_thread.cancel();
        });
        let mut env = std::env::vars().collect::<HashMap<_, _>>();
        env.insert(
            "AGENA_TEST_PID_FILE".to_string(),
            pid_path.to_string_lossy().into_owned(),
        );
        let request = ShellRequest {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "sleep 30 & echo $! > \"$AGENA_TEST_PID_FILE\"; wait".to_string(),
            ],
            cwd: std::env::current_dir().expect("current directory"),
            env,
            timeout_ms: None,
        };

        let result = execute(&request, Some(&cancellation));

        canceller.join().expect("canceller thread");
        assert!(matches!(result, Err(ShellError::Cancelled)));
        let pid = std::fs::read_to_string(&pid_path)
            .expect("shell should publish descendant pid before cancellation")
            .trim()
            .parse::<i32>()
            .expect("valid descendant pid");
        let reaped = Instant::now();
        while process_exists(pid) && reaped.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(10));
        }
        if process_exists(pid) {
            // SAFETY: best-effort cleanup for a failed assertion.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            panic!("shell descendant {pid} survived process-group cancellation");
        }
        let _ = std::fs::remove_file(pid_path);
    }
}
