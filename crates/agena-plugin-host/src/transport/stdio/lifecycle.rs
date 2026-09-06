//! One supervisor owns each child, independently of its stdout and stdin.

use std::process::ExitStatus;

use agena_process::ManagedChild;

use super::*;

// EOF and OS exit can arrive in either order. Allow a short drain/exit window
// without letting an inherited pipe or a daemon that closed stdout live forever.
const EXIT_GRACE: Duration = Duration::from_secs(1);
const REAP_TIMEOUT: Duration = Duration::from_secs(3);
const TASK_STOP_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) enum ReaderEnd {
    Eof,
    Failure(String),
}

pub(super) struct ChildTasks {
    pub reader: oneshot::Receiver<ReaderEnd>,
    pub writer: oneshot::Receiver<String>,
    pub stop: CancellationToken,
    pub tracker: TaskTracker,
}

#[derive(Default)]
struct ExitOutcome {
    status: Option<ExitStatus>,
    failure: Option<String>,
    cleanup_failed: bool,
}

impl ExitOutcome {
    fn record_failure(&mut self, message: impl Into<String>) {
        let message = message.into();
        match &mut self.failure {
            Some(existing) => {
                existing.push_str("; ");
                existing.push_str(&message);
            }
            None => self.failure = Some(message),
        }
    }

    fn record_status(&mut self, result: std::io::Result<ExitStatus>) {
        match result {
            Ok(status) => self.status = Some(status),
            Err(error) => self.record_failure(format!("wait for stdio plugin process: {error}")),
        }
    }

    fn record_reader(&mut self, result: Result<ReaderEnd, oneshot::error::RecvError>) {
        match result {
            Ok(ReaderEnd::Eof) => {}
            Ok(ReaderEnd::Failure(error)) => self.record_failure(error),
            Err(error) => {
                self.record_failure(format!("stdio stdout reader stopped unexpectedly: {error}"))
            }
        }
    }
}

impl Inner {
    pub(super) async fn supervise_child(
        self: Arc<Self>,
        generation: u64,
        mut child: ManagedChild,
        tasks: ChildTasks,
        finished: oneshot::Sender<Result<(), String>>,
    ) {
        let ChildTasks {
            mut reader,
            mut writer,
            stop,
            tracker,
        } = tasks;
        let mut outcome = ExitOutcome::default();
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => {}
            _ = stop.cancelled() => outcome.record_failure("stdio process generation stopped during initialization"),
            result = child.wait() => {
                outcome.record_status(result);
                // Preserve complete response frames written just before exit.
                // A descendant may keep the pipe open, so draining is bounded.
                tokio::select! {
                    biased;
                    _ = self.shutdown.cancelled() => {}
                    result = &mut reader => outcome.record_reader(result),
                    _ = tokio::time::sleep(EXIT_GRACE) => outcome.record_failure("stdio stdout remained open after the child exited"),
                }
            }
            result = &mut reader => {
                let eof = matches!(&result, Ok(ReaderEnd::Eof));
                outcome.record_reader(result);
                if eof {
                    tokio::select! {
                        biased;
                        _ = self.shutdown.cancelled() => {}
                        result = child.wait() => outcome.record_status(result),
                        _ = tokio::time::sleep(EXIT_GRACE) => outcome.record_failure("stdio stdout closed while the child remained running"),
                    }
                }
            }
            result = &mut writer => {
                outcome.record_failure(result.unwrap_or_else(|error| format!("stdio stdin writer stopped unexpectedly: {error}")));
            }
        }

        // Cancel old callbacks and pipe tasks before allowing a new process
        // to appear. ManagedChild also terminates any surviving descendants.
        stop.cancel();
        let mut cleanup = Ok(());
        let kill_error = child.start_kill().err();
        match tokio::time::timeout(REAP_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => {
                if outcome.status.is_none() {
                    outcome.status = Some(status);
                }
            }
            result => {
                let message = match result {
                    Ok(Err(error)) => format!("reap stdio plugin process: {error}"),
                    Err(error) => format!("timed out reaping stdio plugin process: {error}"),
                    Ok(Ok(_)) => unreachable!(),
                };
                outcome.record_failure(message.clone());
                cleanup = Err(message);
            }
        }
        // A process exiting concurrently can reject a group signal before
        // wait has reaped it (including EPERM on macOS). Retry after wait;
        // only a successful retry establishes that cleanup is complete.
        if let Some(first_error) = kill_error
            && let Err(error) = child.start_kill()
        {
            let message = format!(
                "terminate stdio plugin process tree: {first_error}; retry after wait: {error}"
            );
            outcome.record_failure(message.clone());
            cleanup = Err(message);
        }
        drop(child);
        if let Err(error) = tokio::time::timeout(TASK_STOP_TIMEOUT, tracker.wait()).await {
            let message = format!("stdio plugin tasks did not stop after cancellation: {error}");
            outcome.record_failure(message.clone());
            cleanup = Err(message);
        }
        outcome.cleanup_failed = cleanup.is_err();
        if let Err(error) = self.dispose_hosted_generation(generation).await {
            outcome.record_failure(format!("dispose stdio process registrations: {error}"));
            outcome.cleanup_failed = true;
            cleanup = Err(error);
        }
        if self.closed.load(Ordering::SeqCst)
            && let Err(error) = &cleanup
        {
            self.record_spawn_failure(error);
        }
        // close() may own spawn_lock while waiting for this acknowledgment.
        // Signal cleanup before taking that lock for lifecycle publication.
        let _ = finished.send(cleanup);
        self.handle_child_exit(generation, outcome).await;
    }

    async fn handle_child_exit(self: Arc<Self>, generation: u64, mut outcome: ExitOutcome) {
        let spawn_guard = self.spawn_lock.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut handles = self.handles.lock().await;
            if !handles
                .as_ref()
                .is_some_and(|handles| handles.generation == generation)
            {
                return;
            }
            handles.take();
        }
        self.fail_pending("plugin disconnected");
        self.fail_active_streams(PluginError::internal("plugin disconnected"))
            .await;

        let exit_code = outcome.status.and_then(|status| status.code());
        let failed =
            outcome.failure.is_some() || !outcome.status.is_some_and(|status| status.success());
        if failed && outcome.failure.is_none() {
            outcome.record_failure(match outcome.status {
                Some(status) => format!("stdio plugin exited with {status}"),
                None => "stdio plugin exited without an available process status".into(),
            });
        }
        let will_restart = !outcome.cleanup_failed
            && match self.restart_policy.policy {
                RestartMode::Never => false,
                RestartMode::OnFailure => failed,
                RestartMode::Always => true,
            };
        self.record_status(|sink, plugin_id| {
            sink.record_exit(plugin_id, will_restart, exit_code, outcome.failure.clone())
        });
        self.record_log(
            if failed { "warn" } else { "info" },
            "host",
            outcome
                .failure
                .unwrap_or_else(|| "plugin exited successfully".into()),
            serde_json::json!({"exit_code": exit_code, "will_restart": will_restart}),
        );
        drop(spawn_guard);
        if will_restart && let Err(error) = Self::spawn_child(&self, true).await {
            tracing::warn!(target: "agena_plugin_host::stdio", %error, "stdio plugin respawn failed");
        }
    }
}
