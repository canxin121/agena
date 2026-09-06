use std::{fmt::Display, future::Future, marker::PhantomData, sync::Arc};

use chrono::Utc;
use futures_util::FutureExt;
use parking_lot::Mutex;
use std::panic::AssertUnwindSafe;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskCompletion, RuntimeBackgroundTaskControlError,
    RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskSpec,
    RuntimeBackgroundTaskStart, RuntimeBackgroundTaskState, RuntimeBackgroundTaskStatus,
};

const DEFAULT_TASK_HISTORY_LIMIT: usize = 64;
pub(crate) const DEFAULT_ACTIVE_TASK_LIMIT: usize = 64;

#[cfg(test)]
mod tests;

/// Observer hook called when a runtime background task starts or reaches a
/// terminal state. Lets the runtime surface maintenance tasks through the
/// unified background-activity registry without coupling the registry to it.
#[allow(unused_variables)]
pub trait RuntimeBackgroundTaskListener: Send + Sync {
    fn on_started(&self, task: &RuntimeBackgroundTask) {}
    fn on_finished(&self, task: &RuntimeBackgroundTask) {}
}

/// Runtime-owned registry algorithm parameterized by the caller's error type.
pub(crate) struct RuntimeBackgroundTaskRegistry<E> {
    inner: Arc<Mutex<RuntimeBackgroundTaskState>>,
    history_limit: usize,
    active_limit: usize,
    marker: PhantomData<fn() -> E>,
}

impl<E> Clone for RuntimeBackgroundTaskRegistry<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            history_limit: self.history_limit,
            active_limit: self.active_limit,
            marker: PhantomData,
        }
    }
}

impl<E> Default for RuntimeBackgroundTaskRegistry<E> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeBackgroundTaskState::default())),
            history_limit: DEFAULT_TASK_HISTORY_LIMIT,
            active_limit: DEFAULT_ACTIVE_TASK_LIMIT,
            marker: PhantomData,
        }
    }
}

impl<E> RuntimeBackgroundTaskRegistry<E> {
    /// Attach an observer notified when tasks start or finish. Only one
    /// listener is supported per registry; later calls replace the previous
    /// one.
    pub(crate) fn set_listener(&self, listener: Arc<dyn RuntimeBackgroundTaskListener>) {
        let previous = self.inner.lock().listener.replace(listener);
        drop(previous);
    }

    pub(crate) fn list(&self) -> Vec<RuntimeBackgroundTask> {
        let state = self.inner.lock();
        state
            .order
            .iter()
            .filter_map(|task_id| state.tasks.get(task_id).cloned())
            .collect()
    }

    pub(crate) fn is_kind_running(&self, kind: RuntimeBackgroundTaskKind) -> bool {
        let state = self.inner.lock();
        state
            .tasks
            .values()
            .any(|task| task.kind == kind && task.is_running())
    }

    /// Close admission atomically with registration, then cancel all accepted work.
    pub(crate) fn shutdown(&self) {
        let tokens = {
            let mut state = self.inner.lock();
            state.shutdown = true;
            state.controls.values().cloned().collect::<Vec<_>>()
        };
        for token in tokens {
            token.cancel();
        }
    }

    pub(crate) fn cancel(
        &self,
        task_id: &str,
    ) -> Result<RuntimeBackgroundTask, RuntimeBackgroundTaskControlError> {
        let (task, token) =
            {
                let mut state = self.inner.lock();
                let task = state.tasks.get_mut(task_id).ok_or_else(|| {
                    RuntimeBackgroundTaskControlError::NotFound(task_id.to_owned())
                })?;
                if !task.is_running() {
                    return Err(RuntimeBackgroundTaskControlError::NotRunning(
                        task_id.to_owned(),
                    ));
                }
                if !task.cancellable {
                    return Err(RuntimeBackgroundTaskControlError::NotCancellable(
                        task_id.to_owned(),
                    ));
                }
                task.message = Some("Cancellation requested.".to_owned());
                let task = task.clone();
                let token = state.controls.get(task_id).cloned().ok_or_else(|| {
                    RuntimeBackgroundTaskControlError::NotRunning(task_id.to_owned())
                })?;
                (task, token)
            };
        token.cancel();
        Ok(task)
    }

    pub(crate) fn spawn<F, Fut>(
        &self,
        spec: RuntimeBackgroundTaskSpec,
        work: F,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>
    where
        E: Display + Send + 'static,
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<RuntimeBackgroundTaskOutcome, E>> + Send + 'static,
    {
        let (task, token, executor, listener) = {
            let mut state = self.inner.lock();
            if state.shutdown {
                return Err(RuntimeBackgroundTaskControlError::Shutdown);
            }
            if let Some(dedupe_key) = spec.dedupe_key()
                && let Some(existing_id) = state.active_by_key.get(dedupe_key)
                && let Some(existing) = state.tasks.get(existing_id)
                && existing.is_running()
            {
                return Ok(RuntimeBackgroundTaskStart {
                    started: false,
                    task: existing.clone(),
                });
            }

            if state.controls.len() >= self.active_limit {
                return Err(RuntimeBackgroundTaskControlError::Capacity {
                    limit: self.active_limit,
                });
            }
            let executor = tokio::runtime::Handle::try_current()
                .map_err(|_| RuntimeBackgroundTaskControlError::ExecutorUnavailable)?;
            let now = Utc::now();
            let task = RuntimeBackgroundTask {
                id: format!("rtask_{}", Uuid::new_v4().simple()),
                kind: spec.kind(),
                origin: spec.origin(),
                title: spec.title().to_owned(),
                status: RuntimeBackgroundTaskStatus::Running,
                message: None,
                failure: None,
                created_at: now,
                started_at: now,
                finished_at: None,
                cancellable: spec.cancellable(),
            };
            let token = CancellationToken::new();
            state.order.push_front(task.id.clone());
            state.tasks.insert(task.id.clone(), task.clone());
            state.controls.insert(task.id.clone(), token.clone());
            if let Some(dedupe_key) = spec.dedupe_key().map(ToOwned::to_owned) {
                state
                    .active_by_key
                    .insert(dedupe_key.clone(), task.id.clone());
                state.dedupe_keys.insert(task.id.clone(), dedupe_key);
            }
            state.trim_history(self.history_limit);
            (task, token, executor, state.listener.clone())
        };

        // Observers may inspect/cancel tasks. Never call them while locked,
        // and never let an observer panic strand an accepted task.
        if let Some(listener) = listener {
            notify_listener(&listener, &task, true);
        }
        let (start_tx, start_rx) = tokio::sync::oneshot::channel();
        let task_id = task.id.clone();
        let mut finalizer = WorkerFinalizer {
            registry: Arc::downgrade(&self.inner),
            history_limit: self.history_limit,
            task_id: Some(task_id.clone()),
        };
        let worker = executor.spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            let outcome = AssertUnwindSafe(async {
                tokio::select! {
                    biased;
                    _ = token.cancelled() => RuntimeBackgroundTaskCompletion::Cancelled {
                        message: Some("Cancellation requested.".to_owned()),
                    },
                    // The factory itself may perform work or panic. Construct it
                    // lazily only after checking cancellation, inside the boundary.
                    result = async { work(token.clone()).await } => match result {
                        Ok(RuntimeBackgroundTaskOutcome::Succeeded { message }) =>
                            RuntimeBackgroundTaskCompletion::Succeeded { message },
                        Ok(RuntimeBackgroundTaskOutcome::Cancelled { message }) =>
                            RuntimeBackgroundTaskCompletion::Cancelled { message },
                        Err(error) => failed_task(&task_id, &error.to_string()),
                    },
                }
            })
            .catch_unwind()
            .await;
            let completion = outcome.unwrap_or_else(|payload| {
                failed_task(
                    &task_id,
                    &format!(
                        "background task panicked: {}",
                        panic_message(payload.as_ref())
                    ),
                )
            });
            finalizer.finish(completion);
        });
        {
            let mut state = self.inner.lock();
            // Executor shutdown can drop the worker before registration here.
            // Its finalizer has already removed the control in that case.
            if state.controls.contains_key(&task.id) {
                state.workers.insert(task.id.clone(), worker.abort_handle());
            }
        }
        if start_tx.send(()).is_err() {
            tracing::debug!(task_id = %task.id, "background task stopped before its start acknowledgement");
        }
        Ok(RuntimeBackgroundTaskStart {
            started: true,
            task,
        })
    }
}

struct WorkerFinalizer {
    registry: std::sync::Weak<Mutex<RuntimeBackgroundTaskState>>,
    history_limit: usize,
    task_id: Option<String>,
}

impl WorkerFinalizer {
    fn finish(&mut self, completion: RuntimeBackgroundTaskCompletion) {
        if let Some(task_id) = self.task_id.take()
            && let Some(registry) = self.registry.upgrade()
        {
            finish_task(&registry, self.history_limit, &task_id, completion);
        }
    }
}

impl Drop for WorkerFinalizer {
    fn drop(&mut self) {
        self.finish(RuntimeBackgroundTaskCompletion::Cancelled {
            message: Some("Background worker stopped before completion.".to_owned()),
        });
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn failed_task(task_id: &str, diagnostic: &str) -> RuntimeBackgroundTaskCompletion {
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new("background_task.internal"),
        agena_failure::FailureCategory::Internal,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::Unknown,
        agena_failure::RecoveryDirective::Retry,
        agena_failure::FailureImpact::BackgroundTaskFailed,
        agena_failure::UserPresentation::validated_with_context(
            "background-task-failed",
            diagnostic,
        ),
    );
    tracing::error!(task_id, failure_id = %failure.id, diagnostic, "background task failed");
    RuntimeBackgroundTaskCompletion::Failed { failure }
}

fn notify_listener(
    listener: &Arc<dyn RuntimeBackgroundTaskListener>,
    task: &RuntimeBackgroundTask,
    started: bool,
) {
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        if started {
            listener.on_started(task);
        } else {
            listener.on_finished(task);
        }
    }));
    if let Err(payload) = outcome {
        tracing::error!(task_id = %task.id, started, diagnostic = panic_message(payload.as_ref()), "background task observer panicked");
    }
}

fn finish_task(
    inner: &Arc<Mutex<RuntimeBackgroundTaskState>>,
    history_limit: usize,
    task_id: &str,
    completion: RuntimeBackgroundTaskCompletion,
) {
    let (finished, listener) = {
        let mut state = inner.lock();
        let Some(task) = state
            .tasks
            .get_mut(task_id)
            .filter(|task| task.is_running())
        else {
            return;
        };
        task.finished_at = Some(Utc::now());
        match completion {
            RuntimeBackgroundTaskCompletion::Succeeded { message } => {
                task.status = RuntimeBackgroundTaskStatus::Succeeded;
                task.message = message;
                task.failure = None;
            }
            RuntimeBackgroundTaskCompletion::Failed { failure } => {
                task.status = RuntimeBackgroundTaskStatus::Failed;
                task.message = None;
                task.failure = Some(failure);
            }
            RuntimeBackgroundTaskCompletion::Cancelled { message } => {
                task.status = RuntimeBackgroundTaskStatus::Cancelled;
                task.message = message;
                task.failure = None;
            }
        }
        let finished = task.clone();
        state.controls.remove(task_id);
        state.workers.remove(task_id);
        if let Some(dedupe_key) = state.dedupe_keys.remove(task_id)
            && state
                .active_by_key
                .get(&dedupe_key)
                .is_some_and(|id| id == task_id)
        {
            state.active_by_key.remove(&dedupe_key);
        }
        state.trim_history(history_limit);
        (finished, state.listener.clone())
    };
    if let Some(listener) = listener {
        notify_listener(&listener, &finished, false);
    }
}
