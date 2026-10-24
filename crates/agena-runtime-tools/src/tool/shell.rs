//! Tokio-native foreground shell executor.
//!
//! Process lifecycle, pipe draining, timeout, and cancellation all run on the
//! Tokio runtime. Filesystem and tool permissions remain runtime-owned; this
//! opt-in OS sandbox wrapping is applied before this runner by shell_sandbox.

use std::collections::HashMap;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use agena_process::ManagedChild;
use agena_tool::{ShellError, ShellOutput, ShellRequest};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::ToolError;

const MAX_CAPTURE_BYTES_PER_STREAM: usize = 1024 * 1024;
const MAX_CONCURRENT_SHELL_WORKERS: usize = 16;

/// Decode arbitrary pipe segments without corrupting a character split across
/// reads. Only an incomplete final code point is retained (at most 3 bytes).
pub(crate) fn decode_output(pending: &mut Vec<u8>, bytes: &[u8], eof: bool) -> String {
    pending.extend_from_slice(bytes);
    let mut text = String::new();
    let mut consumed = 0;
    while consumed < pending.len() {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(valid) => {
                text.push_str(valid);
                consumed = pending.len();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                text.push_str(
                    std::str::from_utf8(&pending[consumed..consumed + valid])
                        .expect("validated UTF-8 prefix"),
                );
                consumed += valid;
                if let Some(invalid) = error.error_len() {
                    text.push('\u{fffd}');
                    consumed += invalid;
                } else {
                    if eof {
                        text.push_str(&String::from_utf8_lossy(&pending[consumed..]));
                        consumed = pending.len();
                    }
                    break;
                }
            }
        }
    }
    // Retain only an incomplete trailing code point, shifting the buffer once.
    pending.drain(..consumed);
    text
}

static SHELL_WORKERS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SHELL_WORKERS)));

pub(crate) async fn acquire_worker_permit() -> Result<tokio::sync::OwnedSemaphorePermit, ToolError>
{
    Arc::clone(&SHELL_WORKERS)
        .acquire_owned()
        .await
        .map_err(|error| {
            ToolError::plugin(agena_failure::diagnostic::format_error_chain_with_context(
                "acquire a shell worker permit",
                &error,
            ))
        })
}

/// Run a foreground command and, when `live` is attached, report every chunk it
/// produces while it is still running. The returned [`ShellOutput`] stays the
/// single source of truth for the terminal result.
pub(crate) async fn execute_with_sink(
    request: &ShellRequest,
    cancellation: Option<&CancellationToken>,
    live: Option<crate::tool::ToolOutputStreams>,
) -> Result<ShellOutput, ShellError> {
    validate(request).await?;

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
    let mut child = agena_process::spawn(command).map_err(ShellError::Spawn)?;
    let stdout_handle = child.stdout().take().map(|reader| {
        spawn_drain_channel(
            reader,
            live.clone(),
            agena_domain::CommandOutputStream::Stdout,
        )
    });
    let stderr_handle = child.stderr().take().map(|reader| {
        spawn_drain_channel(
            reader,
            live.clone(),
            agena_domain::CommandOutputStream::Stderr,
        )
    });
    struct DrainGuard(Vec<tokio::task::AbortHandle>);
    impl Drop for DrainGuard {
        fn drop(&mut self) {
            for reader in &self.0 {
                reader.abort();
            }
        }
    }
    let _drains = DrainGuard(
        stdout_handle
            .iter()
            .chain(stderr_handle.iter())
            .map(tokio::task::JoinHandle::abort_handle)
            .collect(),
    );

    let timeout = async {
        match request.timeout_ms {
            Some(timeout_ms) => tokio::time::sleep(Duration::from_millis(timeout_ms)).await,
            None => std::future::pending::<()>().await,
        }
    };
    let cancelled = async {
        match cancellation {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(timeout);
    tokio::pin!(cancelled);

    let wait_outcome = tokio::select! {
        biased;
        _ = &mut cancelled => {
            terminate_process_tree(&mut child).await?;
            WaitOutcome::Cancelled
        }
        _ = &mut timeout => {
            let status = terminate_process_tree(&mut child).await?;
            WaitOutcome::TimedOut(status)
        }
        result = child.wait() => {
            WaitOutcome::Exited(result.map_err(ShellError::Wait)?)
        }
    };

    // The direct command may exit while a detached descendant still owns an
    // inherited pipe. `process-wrap` targets the complete group/job here.
    child.start_kill().map_err(ShellError::Wait)?;
    let (stdout, stderr) = match collect_drains(stdout_handle, stderr_handle).await {
        Ok(output) => output,
        Err(error) => {
            if let Some(output) = &live {
                output.record_loss(
                    agena_domain::ContentKind::Log,
                    0,
                    &format!("pipe capture did not finish: {error}"),
                );
            }
            return Err(error);
        }
    };

    if matches!(wait_outcome, WaitOutcome::Cancelled) {
        return Err(ShellError::Cancelled);
    }

    let duration = started.elapsed();
    let (status, timed_out) = match wait_outcome {
        WaitOutcome::Exited(status) => (status, false),
        WaitOutcome::TimedOut(status) => (status, true),
        WaitOutcome::Cancelled => unreachable!("cancelled outcome returned above"),
    };
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

async fn validate(request: &ShellRequest) -> Result<(), ShellError> {
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
    let metadata = tokio::fs::metadata(&request.cwd).await.map_err(|_| {
        ShellError::InvalidRequest(format!(
            "shell cwd does not exist: {}",
            request.cwd.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(ShellError::InvalidRequest(format!(
            "shell cwd is not a directory: {}",
            request.cwd.display()
        )));
    }
    Ok(())
}

/// Strip environment variables that can hijack a child shell or loader.
pub(crate) fn sanitize_env(env: &HashMap<String, String>) -> HashMap<String, String> {
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
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

#[derive(Debug, Clone, Copy)]
enum WaitOutcome {
    Exited(ExitStatus),
    TimedOut(ExitStatus),
    Cancelled,
}

async fn terminate_process_tree(child: &mut ManagedChild) -> Result<ExitStatus, ShellError> {
    child
        .terminate(Duration::from_millis(150))
        .await
        .map_err(ShellError::Wait)
}

fn status_to_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.signal().map(|signal| 128 + signal).unwrap_or(-1)
        }
        #[cfg(not(unix))]
        {
            -1
        }
    })
}

fn spawn_drain_channel<R>(
    reader: R,
    live: Option<crate::tool::ToolOutputStreams>,
    stream: agena_domain::CommandOutputStream,
) -> tokio::task::JoinHandle<io::Result<String>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = reader;
        let half = MAX_CAPTURE_BYTES_PER_STREAM / 2;
        let mut captured_output = Vec::new();
        let mut captured_tail = std::collections::VecDeque::<u8>::new();
        let mut captured_bytes = 0usize;
        let mut pending_utf8 = Vec::new();
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let read = match reader.read(&mut chunk).await {
                Ok(read) => read,
                Err(error) => {
                    if let Some(live) = &live {
                        live.record_loss(
                            agena_domain::ContentKind::Log,
                            0,
                            &format!("output pipe read failed: {error}"),
                        );
                    }
                    return Err(error);
                }
            };
            if read == 0 {
                break;
            }
            captured_bytes = captured_bytes.saturating_add(read);
            let head_bytes = half.saturating_sub(captured_output.len()).min(read);
            captured_output.extend_from_slice(&chunk[..head_bytes]);
            captured_tail.extend(&chunk[head_bytes..read]);
            if captured_tail.len() > half {
                captured_tail.drain(..captured_tail.len() - half);
            }
            if let Some(live) = live.as_ref() {
                // Display keeps advancing even after the stored result hits
                // its cap; stdout/stderr still drain independently.
                let text = decode_output(&mut pending_utf8, &chunk[..read], false);
                let bytes = text.len();
                if !text.is_empty()
                    && let Err(error) = live.try_log(stream.clone(), &text)
                {
                    live.record_loss(agena_domain::ContentKind::Log, bytes, &error.to_string());
                }
            }
        }
        if let Some(live) = live.as_ref() {
            let text = decode_output(&mut pending_utf8, &[], true);
            let bytes = text.len();
            if !text.is_empty()
                && let Err(error) = live.try_log(stream.clone(), &text)
            {
                live.record_loss(agena_domain::ContentKind::Log, bytes, &error.to_string());
            }
        }
        if captured_bytes > MAX_CAPTURE_BYTES_PER_STREAM {
            captured_output.extend_from_slice(
                b"\n[output truncated: retained beginning and end within 1 MiB]\n",
            );
        }
        captured_output.extend(captured_tail);
        Ok(String::from_utf8_lossy(&captured_output).into_owned())
    })
}

async fn collect_drains(
    mut stdout_handle: Option<tokio::task::JoinHandle<io::Result<String>>>,
    mut stderr_handle: Option<tokio::task::JoinHandle<io::Result<String>>>,
) -> Result<(String, String), ShellError> {
    let stdout_abort = stdout_handle
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
    let stderr_abort = stderr_handle
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
    match tokio::time::timeout(Duration::from_secs(2), async {
        let stdout = async {
            match stdout_handle.as_mut() {
                Some(handle) => handle.await,
                None => Ok(Ok(String::new())),
            }
        };
        let stderr = async {
            match stderr_handle.as_mut() {
                Some(handle) => handle.await,
                None => Ok(Ok(String::new())),
            }
        };
        tokio::join!(stdout, stderr)
    })
    .await
    {
        Ok((stdout, stderr)) => {
            let mut failures = Vec::new();
            let stdout = match stdout {
                Ok(Ok(output)) => Some(output),
                Ok(Err(error)) => {
                    failures.push(agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to drain shell stdout",
                        &error,
                    ));
                    None
                }
                Err(error) => {
                    failures.push(agena_failure::diagnostic::format_error_chain_with_context(
                        "shell stdout drain task failed",
                        &error,
                    ));
                    None
                }
            };
            let stderr = match stderr {
                Ok(Ok(output)) => Some(output),
                Ok(Err(error)) => {
                    failures.push(agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to drain shell stderr",
                        &error,
                    ));
                    None
                }
                Err(error) => {
                    failures.push(agena_failure::diagnostic::format_error_chain_with_context(
                        "shell stderr drain task failed",
                        &error,
                    ));
                    None
                }
            };
            if failures.is_empty() {
                Ok((
                    stdout.expect("stdout is present when no drain failure was recorded"),
                    stderr.expect("stderr is present when no drain failure was recorded"),
                ))
            } else {
                Err(ShellError::Wait(io::Error::other(
                    failures.join("; additionally, "),
                )))
            }
        }
        Err(timeout_error) => {
            if let Some(abort) = stdout_abort {
                abort.abort();
            }
            if let Some(abort) = stderr_abort {
                abort.abort();
            }
            let mut diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                "shell output drains did not stop within 2 seconds after process termination",
                &timeout_error,
            );
            for (stream, task) in [
                ("stdout", stdout_handle.take()),
                ("stderr", stderr_handle.take()),
            ] {
                if let Some(task) = task
                    && let Err(error) = task.await
                    && !error.is_cancelled()
                {
                    diagnostic.push_str("; additionally, ");
                    diagnostic.push_str(
                        &agena_failure::diagnostic::format_error_chain_with_context(
                            format!("shell {stream} drain did not stop cleanly after abort"),
                            &error,
                        ),
                    );
                }
            }
            Err(ShellError::Wait(io::Error::new(
                io::ErrorKind::TimedOut,
                diagnostic,
            )))
        }
    }
}
