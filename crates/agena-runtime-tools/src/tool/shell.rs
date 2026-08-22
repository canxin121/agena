//! Tokio-native foreground shell executor.
//!
//! Process lifecycle, pipe draining, timeout, and cancellation all run on the
//! Tokio runtime. Filesystem and tool permissions remain runtime-owned; this
//! module does not attempt to provide an OS sandbox.

use std::collections::HashMap;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use agena_domain::CommandOutputStream;
use agena_process::ManagedChild;
use agena_tool::{ShellError, ShellOutput, ShellRequest};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::ToolError;

const OUTPUT_CHUNK_QUEUE_CAPACITY: usize = 128;
const MAX_CAPTURE_BYTES_PER_STREAM: usize = 8 * 1024 * 1024;
const MAX_CONCURRENT_SHELL_WORKERS: usize = 16;
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

pub async fn execute(
    request: &ShellRequest,
    cancellation: Option<&CancellationToken>,
) -> Result<ShellOutput, ShellError> {
    execute_with_callback(request, cancellation, None).await
}

/// Run a command while forwarding bounded stdout/stderr chunks to a callback.
/// Pipe I/O and process waiting remain asynchronous; the callback must be
/// non-blocking because it executes on the current Tokio task.
pub async fn execute_with_callback(
    request: &ShellRequest,
    cancellation: Option<&CancellationToken>,
    output_callback: Option<&(dyn Fn(CommandOutputStream, &[u8]) + Send + Sync)>,
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

    let started = Instant::now();
    let mut child = agena_process::spawn(command).map_err(ShellError::Spawn)?;
    let (chunk_tx, mut chunk_rx) = mpsc::channel(OUTPUT_CHUNK_QUEUE_CAPACITY);
    let stdout_handle = child
        .stdout()
        .take()
        .map(|reader| spawn_drain(reader, CommandOutputStream::Stdout, chunk_tx.clone()));
    let stderr_handle = child
        .stderr()
        .take()
        .map(|reader| spawn_drain(reader, CommandOutputStream::Stderr, chunk_tx.clone()));
    drop(chunk_tx);

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

    let mut chunks_open = true;
    let mut output_sequence = 0_u64;
    let wait_outcome = loop {
        tokio::select! {
            biased;
            _ = &mut cancelled => {
                terminate_process_tree(&mut child).await?;
                break WaitOutcome::Cancelled;
            }
            _ = &mut timeout => {
                let status = terminate_process_tree(&mut child).await?;
                break WaitOutcome::TimedOut(status);
            }
            chunk = chunk_rx.recv(), if chunks_open => {
                match chunk {
                    Some(chunk) => emit_chunk(
                        chunk,
                        output_callback,
                        &mut output_sequence,
                    ),
                    None => chunks_open = false,
                }
            }
            result = child.wait() => {
                break WaitOutcome::Exited(result.map_err(ShellError::Wait)?);
            }
        }
    };

    // The direct command may exit while a detached descendant still owns an
    // inherited pipe. `process-wrap` targets the complete group/job here.
    child.start_kill().map_err(ShellError::Wait)?;
    let (stdout, stderr) = collect_drains(stdout_handle, stderr_handle).await?;
    while let Ok(chunk) = chunk_rx.try_recv() {
        emit_chunk(chunk, output_callback, &mut output_sequence);
    }

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

/// Strip environment variables that can hijack a child shell or loader.
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

struct OutputChunk {
    stream: CommandOutputStream,
    bytes: Vec<u8>,
}

fn spawn_drain<R>(
    mut reader: R,
    stream: CommandOutputStream,
    sender: mpsc::Sender<OutputChunk>,
) -> tokio::task::JoinHandle<io::Result<String>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut captured_output = Vec::new();
        let mut truncated = false;
        let mut live_delivery_partial = false;
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let read = reader.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            let remaining = MAX_CAPTURE_BYTES_PER_STREAM.saturating_sub(captured_output.len());
            let captured = remaining.min(read);
            captured_output.extend_from_slice(&chunk[..captured]);
            truncated |= captured < read;
            if let Err(error) = sender.try_send(OutputChunk {
                stream: stream.clone(),
                bytes: chunk[..read].to_vec(),
            }) && !live_delivery_partial
            {
                live_delivery_partial = true;
                tracing::warn!(
                    output_stream = ?stream,
                    diagnostic = %error,
                    "live shell output chunk was not delivered; the terminal stream is partial but captured output remains available"
                );
            }
        }
        if truncated {
            captured_output.extend_from_slice(b"\n[output truncated after 8 MiB]\n");
        }
        Ok(String::from_utf8_lossy(&captured_output).into_owned())
    })
}

fn emit_chunk(
    chunk: OutputChunk,
    output_callback: Option<&(dyn Fn(CommandOutputStream, &[u8]) + Send + Sync)>,
    output_sequence: &mut u64,
) {
    let Some(output_callback) = output_callback else {
        return;
    };
    *output_sequence = output_sequence.saturating_add(1);
    output_callback(chunk.stream, chunk.bytes.as_slice());
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn process_exists(pid: i32) -> bool {
        // SAFETY: signal 0 performs existence/permission checking only.
        (unsafe { libc::kill(pid, 0) == 0 })
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_stops_a_foreground_process_group_promptly() {
        let cancellation = CancellationToken::new();
        let cancel_from_task = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_from_task.cancel();
        });
        let request = ShellRequest {
            command: vec!["sh".to_string(), "-c".to_string(), "sleep 30".to_string()],
            cwd: std::env::current_dir().expect("current directory"),
            env: std::env::vars().collect(),
            timeout_ms: None,
        };
        let started = Instant::now();

        let result = execute(&request, Some(&cancellation)).await;

        assert!(matches!(result, Err(ShellError::Cancelled)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancellation took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_kills_shell_grandchildren_not_just_the_shell() {
        let pid_path = std::env::temp_dir().join(format!(
            "agena-shell-descendant-{}-{}.pid",
            std::process::id(),
            uuid::Uuid::new_v4().simple(),
        ));
        let cancellation = CancellationToken::new();
        let cancel_from_task = cancellation.clone();
        let pid_path_for_task = pid_path.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            while !pid_path_for_task.exists() && started.elapsed() < Duration::from_secs(2) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            cancel_from_task.cancel();
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

        let result = execute(&request, Some(&cancellation)).await;

        assert!(matches!(result, Err(ShellError::Cancelled)));
        let pid = std::fs::read_to_string(&pid_path)
            .expect("shell should publish descendant pid before cancellation")
            .trim()
            .parse::<i32>()
            .expect("valid descendant pid");
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("shell descendant should be terminated");
        let _ = std::fs::remove_file(pid_path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exiting_shell_cleans_up_background_descendants_and_open_pipes() {
        let pid_path = std::env::temp_dir().join(format!(
            "agena-shell-exit-descendant-{}-{}.pid",
            std::process::id(),
            uuid::Uuid::new_v4().simple(),
        ));
        let mut env = std::env::vars().collect::<HashMap<_, _>>();
        env.insert(
            "AGENA_TEST_PID_FILE".to_string(),
            pid_path.to_string_lossy().into_owned(),
        );
        let request = ShellRequest {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "sleep 30 & echo $! > \"$AGENA_TEST_PID_FILE\"".to_string(),
            ],
            cwd: std::env::current_dir().expect("current directory"),
            env,
            timeout_ms: Some(2_000),
        };

        tokio::time::timeout(Duration::from_secs(2), execute(&request, None))
            .await
            .expect("foreground execution must not wait on inherited descendant pipes")
            .expect("shell execution");
        let pid = std::fs::read_to_string(&pid_path)
            .expect("shell should publish descendant pid")
            .trim()
            .parse::<i32>()
            .expect("valid descendant pid");
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("background descendant should be terminated when the shell exits");
        let _ = std::fs::remove_file(pid_path);
    }
}
