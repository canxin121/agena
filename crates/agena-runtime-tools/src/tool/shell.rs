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

const MAX_CAPTURE_BYTES_PER_STREAM: usize = 8 * 1024 * 1024;
const MAX_CONCURRENT_SHELL_WORKERS: usize = 16;

/// Latest display tail, independent of the bounded result captured at exit.
/// A slow consumer skips intermediate snapshots without blocking pipe drains
/// or growing an unbounded queue. The final unseen snapshot survives closure.
#[derive(Debug, Clone)]
pub struct ShellOutputSink(tokio::sync::watch::Sender<String>);

impl ShellOutputSink {
    pub(crate) fn channel() -> (Self, tokio::sync::watch::Receiver<String>) {
        let (tx, rx) = tokio::sync::watch::channel(String::new());
        (Self(tx), rx)
    }

    fn publish(&self, text: &str) {
        const MAX_LIVE_BYTES: usize = 8 * 1024;
        if text.is_empty() || self.0.is_closed() {
            return;
        }
        self.0.send_if_modified(|tail| {
            if text.len() >= MAX_LIVE_BYTES {
                let mut start = text.len() - MAX_LIVE_BYTES;
                while !text.is_char_boundary(start) {
                    start += 1;
                }
                if tail.as_str() == &text[start..] {
                    return false;
                }
                tail.clear();
                tail.push_str(&text[start..]);
            } else {
                let mut start = (tail.len() + text.len()).saturating_sub(MAX_LIVE_BYTES);
                while !tail.is_char_boundary(start) {
                    start += 1;
                }
                let retained = tail.len() - start;
                if tail.len() == retained + text.len()
                    && tail.as_bytes()[..retained] == tail.as_bytes()[start..]
                    && &tail.as_bytes()[retained..] == text.as_bytes()
                {
                    return false;
                }
                tail.drain(..start);
                tail.push_str(text);
            }
            true
        });
    }
}

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
pub async fn execute_with_sink(
    request: &ShellRequest,
    cancellation: Option<&CancellationToken>,
    live: Option<ShellOutputSink>,
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
    let stdout_handle = child
        .stdout()
        .take()
        .map(|reader| spawn_drain(reader, live.clone()));
    let stderr_handle = child
        .stderr()
        .take()
        .map(|reader| spawn_drain(reader, live.clone()));
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
    let (stdout, stderr) = collect_drains(stdout_handle, stderr_handle).await?;

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

#[cfg(test)]
mod live_output_tests {
    use super::{ShellOutputSink, execute_with_sink};
    use agena_tool::ShellRequest;
    use std::collections::HashMap;

    fn request(script: &str) -> ShellRequest {
        ShellRequest {
            command: vec!["sh".to_string(), "-c".to_string(), script.to_string()],
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            timeout_ms: Some(5_000),
        }
    }

    #[tokio::test]
    async fn live_chunks_arrive_before_the_command_exits() {
        let request = request("printf 'first'; sleep 0.6; printf 'second'");
        let (tx, mut rx) = ShellOutputSink::channel();
        let running =
            tokio::spawn(async move { execute_with_sink(&request, None, Some(tx)).await });

        tokio::time::timeout(std::time::Duration::from_secs(5), rx.changed())
            .await
            .expect("a live chunk arrives while the command still runs")
            .expect("the stream stays open");
        assert!(rx.borrow().contains("first"));
        assert!(
            !running.is_finished(),
            "the command must still be running when its first chunk is delivered"
        );

        let output = running
            .await
            .expect("the execution task joins")
            .expect("the command completes");
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("second"));
    }

    #[tokio::test]
    async fn a_dropped_consumer_does_not_fail_the_command() {
        let request = request("printf 'kept'");
        let (tx, rx) = ShellOutputSink::channel();
        drop(rx);
        let output = execute_with_sink(&request, None, Some(tx))
            .await
            .expect("a dropped live consumer never fails the command");
        assert_eq!(output.stdout, "kept");
    }

    #[test]
    fn pipe_segments_preserve_split_utf8_and_replace_invalid_bytes() {
        let mut pending = Vec::new();
        let emoji = "🙂".as_bytes();
        assert_eq!(super::decode_output(&mut pending, &emoji[..2], false), "");
        assert_eq!(super::decode_output(&mut pending, &emoji[2..], false), "🙂");
        assert_eq!(
            super::decode_output(&mut pending, b"a\xffb\xe4", false),
            "a�b"
        );
        assert_eq!(super::decode_output(&mut pending, &[], true), "�");
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn a_slow_live_reader_sees_the_final_tail_after_the_capture_cap_and_closure() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(8192);
        let (sink, mut live) = ShellOutputSink::channel();
        let drain = super::spawn_drain(reader, Some(sink));
        let bytes = vec![b'x'; super::MAX_CAPTURE_BYTES_PER_STREAM + 1];
        writer.write_all(&bytes).await.unwrap();
        writer
            .write_all("\nlatest output 🙂".as_bytes())
            .await
            .unwrap();
        drop(writer);
        let captured = drain.await.unwrap().unwrap();
        assert!(captured.ends_with("[output truncated after 8 MiB]\n"));
        // The reader deliberately never consumed the stream until it closed.
        live.changed().await.unwrap();
        let tail = live.borrow_and_update().clone();
        assert!(tail.len() <= 8192);
        assert!(tail.ends_with("\nlatest output 🙂"));
        assert!(live.changed().await.is_err());
    }
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

fn spawn_drain<R>(
    mut reader: R,
    live: Option<ShellOutputSink>,
) -> tokio::task::JoinHandle<io::Result<String>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut captured_output = Vec::new();
        let mut truncated = false;
        let mut pending_utf8 = Vec::new();
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
            if let Some(live) = live.as_ref() {
                // Display keeps advancing even after the stored result hits
                // its cap; stdout/stderr still drain independently.
                live.publish(&decode_output(&mut pending_utf8, &chunk[..read], false));
            }
        }
        if let Some(live) = live.as_ref() {
            live.publish(&decode_output(&mut pending_utf8, &[], true));
        }
        if truncated {
            captured_output.extend_from_slice(b"\n[output truncated after 8 MiB]\n");
        }
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

        let result = execute_with_sink(&request, Some(&cancellation), None).await;

        assert!(matches!(result, Err(ShellError::Cancelled)), "{result:?}");
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

        let result = execute_with_sink(&request, Some(&cancellation), None).await;

        assert!(matches!(result, Err(ShellError::Cancelled)), "{result:?}");
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

        tokio::time::timeout(
            Duration::from_secs(2),
            execute_with_sink(&request, None, None),
        )
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
