//! In-tree shell executor that replaces the procwarden sandbox.
//!
//! `tool::shell` provides a small synchronous command runner with a watchdog
//! timeout, stdout/stderr capture, and shell-injection environment scrubbing.
//! It deliberately does *not* implement OS-level sandboxing — agena gates
//! filesystem and tool access through `crate::permission` instead. The
//! `ExecutionPolicy` enum stays as a semantic token so call sites can refuse
//! mutating bash commands under a `ReadOnly` profile, but it no longer maps
//! onto a kernel sandbox.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// High-level execution profile. Drives both the bash classifier (block
/// mutating commands under `ReadOnly`) and the env scrubber (skipped for
/// `DangerFullAccess`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPolicy {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    DangerFullAccess,
}

impl ExecutionPolicy {
    pub const fn read_only() -> Self {
        Self::ReadOnly
    }

    pub const fn workspace_write() -> Self {
        Self::WorkspaceWrite
    }

    pub const fn danger_full_access() -> Self {
        Self::DangerFullAccess
    }

    /// Whether this policy permits writes anywhere on disk. The permission
    /// system still gates per-path; this just controls env scrubbing.
    pub const fn is_unrestricted(&self) -> bool {
        matches!(self, Self::DangerFullAccess)
    }
}

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
pub fn execute(
    request: &ShellRequest,
    policy: &ExecutionPolicy,
) -> Result<ShellOutput, ShellError> {
    validate(request)?;

    let env = sanitize_env(&request.env, policy);
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

/// Strip env vars that can hijack a child shell or dynamic loader. Skipped
/// when policy is `DangerFullAccess` so privileged callers can still pass
/// `LD_PRELOAD` etc. through deliberately.
fn sanitize_env(
    env: &HashMap<String, String>,
    policy: &ExecutionPolicy,
) -> HashMap<String, String> {
    if policy.is_unrestricted() {
        return env.clone();
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn req(command: Vec<&str>, cwd: &std::path::Path) -> ShellRequest {
        ShellRequest {
            command: command.into_iter().map(str::to_string).collect(),
            cwd: cwd.to_path_buf(),
            env: HashMap::new(),
            timeout_ms: None,
        }
    }

    #[test]
    fn empty_command_is_rejected() {
        let cwd = std::env::current_dir().unwrap();
        let req = ShellRequest {
            command: Vec::new(),
            cwd,
            env: HashMap::new(),
            timeout_ms: None,
        };
        let err = execute(&req, &ExecutionPolicy::WorkspaceWrite).unwrap_err();
        assert!(matches!(err, ShellError::InvalidRequest(_)));
    }

    #[test]
    fn missing_cwd_is_rejected() {
        let req = req(
            vec!["true"],
            std::path::Path::new("/this/path/should/not/exist/abc123"),
        );
        let err = execute(&req, &ExecutionPolicy::WorkspaceWrite).unwrap_err();
        assert!(matches!(err, ShellError::InvalidRequest(_)));
    }

    #[test]
    fn echo_round_trip() {
        let cwd = std::env::current_dir().unwrap();
        let req = req(vec!["/bin/sh", "-c", "echo hi"], cwd.as_path());
        let out = execute(&req, &ExecutionPolicy::WorkspaceWrite).unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "hi");
        assert!(out.stderr.is_empty());
        assert!(!out.timed_out);
        assert_eq!(out.aggregated_output.trim(), "hi");
    }

    #[test]
    fn timeout_is_enforced() {
        let cwd = std::env::current_dir().unwrap();
        let req = ShellRequest {
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "sleep 5".to_string(),
            ],
            cwd,
            env: HashMap::new(),
            timeout_ms: Some(150),
        };
        let out = execute(&req, &ExecutionPolicy::WorkspaceWrite).unwrap();
        assert!(out.timed_out, "expected timed_out=true");
        assert!(out.duration < Duration::from_secs(2));
    }

    #[test]
    fn env_sanitizer_strips_loader_vars_under_workspace_write() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        env.insert("LD_PRELOAD".to_string(), "evil.so".to_string());
        env.insert(
            "DYLD_INSERT_LIBRARIES".to_string(),
            "evil.dylib".to_string(),
        );
        env.insert("BASH_FUNC_x".to_string(), "() { :; }".to_string());

        let scrubbed = sanitize_env(&env, &ExecutionPolicy::WorkspaceWrite);
        assert!(scrubbed.contains_key("PATH"));
        assert!(!scrubbed.contains_key("LD_PRELOAD"));
        assert!(!scrubbed.contains_key("DYLD_INSERT_LIBRARIES"));
        assert!(!scrubbed.contains_key("BASH_FUNC_x"));
    }

    #[test]
    fn env_sanitizer_passes_through_under_danger_full_access() {
        let mut env = HashMap::new();
        env.insert("LD_PRELOAD".to_string(), "explicit.so".to_string());
        let kept = sanitize_env(&env, &ExecutionPolicy::DangerFullAccess);
        assert_eq!(kept.get("LD_PRELOAD"), Some(&"explicit.so".to_string()));
    }
}
