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
    live: Option<agena_storage::content::ContentWriter>,
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

#[cfg(test)]
fn spawn_drain<R>(
    reader: R,
    live: Option<agena_storage::content::ContentWriter>,
) -> tokio::task::JoinHandle<io::Result<String>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    spawn_drain_channel(reader, live, agena_domain::CommandOutputStream::Stdout)
}

fn spawn_drain_channel<R>(
    reader: R,
    live: Option<agena_storage::content::ContentWriter>,
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
            let read = reader.read(&mut chunk).await?;
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
                if let Err(error) = live.capture(agena_domain::ContentInput::Log {
                    stream: stream.clone(),
                    text,
                }) {
                    live.record_loss(bytes, &error);
                }
            }
        }
        if let Some(live) = live.as_ref() {
            let text = decode_output(&mut pending_utf8, &[], true);
            let bytes = text.len();
            if let Err(error) = live.capture(agena_domain::ContentInput::Log { stream, text }) {
                live.record_loss(bytes, &error);
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

#[cfg(test)]
mod content_tests {
    use super::*;
    use agena_domain::{CommandOutputStream, ContentKind, ContentPayload};
    use agena_storage::content::ContentHub;

    struct StalledPersistence {
        memory: agena_storage::content::MemoryContentBackend,
        entered: tokio::sync::Notify,
        release: tokio::sync::Semaphore,
        waiting: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl agena_storage::content::ContentBackend for StalledPersistence {
        async fn commit(
            &self,
            resource: agena_domain::ContentResource,
            chunks: &[Arc<agena_domain::ContentChunk>],
        ) -> Result<agena_domain::ContentResource, agena_storage::store::StoreError> {
            if !chunks.is_empty()
                && self
                    .waiting
                    .swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
            self.memory.commit(resource, chunks).await
        }
        async fn read(
            &self,
            id: agena_domain::ContentId,
            after: Option<agena_domain::ContentCursor>,
            max_bytes: usize,
        ) -> Result<agena_domain::ContentPage, agena_storage::store::StoreError> {
            self.memory.read(id, after, max_bytes).await
        }
        async fn restore(
            &self,
            archive: &agena_storage::content::ContentArchive,
        ) -> Result<(), agena_storage::store::StoreError> {
            self.memory.restore(archive).await
        }
        async fn prune(
            &self,
            protected: &std::collections::HashSet<agena_domain::ContentId>,
        ) -> Result<usize, agena_storage::store::StoreError> {
            self.memory.prune(protected).await
        }
        async fn delete(
            &self,
            id: agena_domain::ContentId,
        ) -> Result<(), agena_storage::store::StoreError> {
            self.memory.delete(id).await
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_real_subprocess_drains_both_pipes_while_content_persistence_is_stalled() {
        let backend = Arc::new(StalledPersistence {
            memory: Default::default(),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
            waiting: std::sync::atomic::AtomicBool::new(true),
        });
        let hub = ContentHub::new(
            backend.clone(),
            agena_storage::content::ContentConfig {
                max_resident_bytes: 32 * 1024,
                memory_bytes: 32 * 1024,
                pending_bytes: 32 * 1024,
                chunk_bytes: 8192,
                flush_bytes: 1,
                ..Default::default()
            },
        );
        let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
        let source = writer.clone();
        let process = tokio::spawn(async move {
            execute_with_sink(&request("head -c 2097152 /dev/zero; printf '\\nDRAINED\\n'; head -c 2097152 /dev/zero >&2; printf '\\nSTDERR DRAINED\\n' >&2"), None, Some(source)).await
        });
        tokio::time::timeout(Duration::from_secs(2), backend.entered.notified())
            .await
            .unwrap();
        // The commit remains blocked until after the process has exited. A
        // reader that awaits archival capacity would deadlock on pipe writes.
        let result = tokio::time::timeout(Duration::from_secs(3), process)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.ends_with("\nDRAINED\n"));
        assert!(result.stderr.ends_with("\nSTDERR DRAINED\n"));
        assert!(writer.resource().dropped_bytes > 0);
        backend.release.add_permits(1);
        let resource = writer.finish().await.unwrap();
        assert_eq!(resource.state, agena_domain::ContentState::Interrupted);
        assert_eq!(resource.cursor, resource.committed_cursor);
        assert!(resource.capture_error.is_some());
    }

    fn request(script: &str) -> ShellRequest {
        ShellRequest {
            command: vec!["sh".into(), "-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            timeout_ms: Some(5000),
        }
    }

    #[tokio::test]
    async fn process_output_is_visible_before_exit_and_retains_both_channels() {
        let hub = ContentHub::in_memory();
        let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
        let id = writer.resource().resource_id;
        let mut changes = hub.subscribe(id).unwrap();
        let source = writer.clone();
        let running = tokio::spawn(async move {
            execute_with_sink(
                &request("printf '  first'; sleep 0.5; printf 'second'; printf 'warning' >&2"),
                None,
                Some(source),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), changes.changed())
            .await
            .unwrap()
            .unwrap();
        let live = hub.read(id, None, 1024).await.unwrap();
        assert!(live.chunks.iter().any(|chunk| matches!(&chunk.payload,
            ContentPayload::Log { stream: CommandOutputStream::Stdout, text } if text == "  first")));
        assert!(!running.is_finished());
        let result = running.await.unwrap().unwrap();
        assert_eq!(result.stdout, "  firstsecond");
        assert_eq!(result.stderr, "warning");
        writer.finish().await.unwrap();
        let final_output = hub.read(id, None, 1024).await.unwrap();
        assert!(final_output.chunks.iter().any(|chunk| matches!(&chunk.payload,
            ContentPayload::Log { stream: CommandOutputStream::Stderr, text } if text == "warning")));
    }

    #[tokio::test]
    async fn absent_observers_do_not_change_execution_or_capture() {
        let hub = ContentHub::in_memory();
        let writer = hub.open(1, 2, ContentKind::Log).await.unwrap();
        let result = execute_with_sink(&request("printf 'kept'"), None, Some(writer.clone()))
            .await
            .unwrap();
        assert_eq!(result.stdout, "kept");
        let complete = writer.finish().await.unwrap();
        let page = hub.read(complete.resource_id, None, 1024).await.unwrap();
        assert_eq!(
            page.chunks[0].payload,
            ContentPayload::Log {
                stream: CommandOutputStream::Stdout,
                text: "kept".into(),
            }
        );
    }

    #[test]
    fn pipe_segments_preserve_split_utf8_and_replace_invalid_bytes() {
        let mut pending = Vec::new();
        let emoji = "🙂".as_bytes();
        assert_eq!(decode_output(&mut pending, &emoji[..2], false), "");
        assert_eq!(decode_output(&mut pending, &emoji[2..], false), "🙂");
        assert_eq!(decode_output(&mut pending, b"a\xffb\xe4", false), "a�b");
        assert_eq!(decode_output(&mut pending, &[], true), "�");
    }

    #[tokio::test]
    async fn source_retains_output_beyond_the_tool_capture_limit() {
        use tokio::io::AsyncWriteExt;
        let hub = ContentHub::in_memory();
        let source = hub.open(1, 2, ContentKind::Log).await.unwrap();
        let (mut input, reader) = tokio::io::duplex(8192);
        let drain = spawn_drain(reader, Some(source.clone()));
        input
            .write_all(&vec![b'x'; MAX_CAPTURE_BYTES_PER_STREAM + 1])
            .await
            .unwrap();
        input
            .write_all("\nlatest output 🙂".as_bytes())
            .await
            .unwrap();
        drop(input);
        let captured = drain.await.unwrap().unwrap();
        assert!(captured.contains("[output truncated:"));
        let resource = source.finish().await.unwrap();
        assert!(resource.total_bytes > MAX_CAPTURE_BYTES_PER_STREAM as u64);
        let page = hub
            .read(resource.resource_id, None, 64 * 1024)
            .await
            .unwrap();
        let text = page
            .chunks
            .iter()
            .filter_map(|chunk| match &chunk.payload {
                ContentPayload::Log { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(text.ends_with("\nlatest output 🙂"));
    }
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
