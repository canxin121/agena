//! In-tree shell executor.
//!
//! `tool::shell` provides a small synchronous command runner with a watchdog
//! timeout, stdout/stderr capture, and shell-injection environment scrubbing.
//! It deliberately does *not* implement OS-level sandboxing. Agena gates
//! filesystem and tool access through `crate::permission` instead.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

/// Single shell invocation.
#[derive(Debug, Clone)]
pub struct ShellRequest {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub timeout_ms: Option<u64>,
}

/// Result of a shell invocation.
#[derive(Debug, Clone)]
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub aggregated_output: String,
    pub duration: Duration,
    pub timed_out: bool,
}

#[derive(Debug, Error)]
pub enum ShellError {
    #[error("invalid shell request: {0}")]
    InvalidRequest(String),
    #[error("failed to spawn child process: {0}")]
    Spawn(#[source] io::Error),
    #[error("failed to wait for child process: {0}")]
    Wait(#[source] io::Error),
}

/// Run a command synchronously, scrubbing dangerous loader env vars and
/// enforcing `timeout_ms` via a watchdog thread.
pub fn execute(request: &ShellRequest) -> Result<ShellOutput, ShellError> {
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

    let started = Instant::now();
    let mut child = command.spawn().map_err(ShellError::Spawn)?;

    // Drain stdout / stderr off-thread so a child that fills its pipe
    // buffers cannot deadlock the parent.
    let stdout_handle = child.stdout.take().map(spawn_drain);
    let stderr_handle = child.stderr.take().map(spawn_drain);

    let timeout = request.timeout_ms.map(Duration::from_millis);
    let (status, timed_out) = wait_with_timeout(&mut child, timeout)?;

    let duration = started.elapsed();
    let stdout = collect_drain(stdout_handle);
    let stderr = collect_drain(stderr_handle);

    let exit_code = status.map(status_to_code).unwrap_or(-1);

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
) -> Result<(Option<std::process::ExitStatus>, bool), ShellError> {
    let Some(deadline) = timeout else {
        let status = child.wait().map_err(ShellError::Wait)?;
        return Ok((Some(status), false));
    };

    let started = Instant::now();
    let poll_interval = Duration::from_millis(20);
    loop {
        match child.try_wait().map_err(ShellError::Wait)? {
            Some(status) => return Ok((Some(status), false)),
            None => {
                if started.elapsed() >= deadline {
                    let _ = child.kill();
                    let status = child.wait().map_err(ShellError::Wait)?;
                    return Ok((Some(status), true));
                }
                thread::sleep(poll_interval);
            }
        }
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
