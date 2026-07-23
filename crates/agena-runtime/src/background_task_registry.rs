use std::{fmt::Display, future::Future, marker::PhantomData, sync::Arc};

use chrono::Utc;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskCompletion, RuntimeBackgroundTaskControlError,
    RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskSpec,
    RuntimeBackgroundTaskStart, RuntimeBackgroundTaskState, RuntimeBackgroundTaskStatus,
};

const DEFAULT_TASK_HISTORY_LIMIT: usize = 64;

/// Runtime-owned registry algorithm parameterized by the caller's error type.
pub(crate) struct RuntimeBackgroundTaskRegistry<E> {
    inner: Arc<Mutex<RuntimeBackgroundTaskState>>,
    history_limit: usize,
    marker: PhantomData<fn() -> E>,
}

impl<E> Clone for RuntimeBackgroundTaskRegistry<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            history_limit: self.history_limit,
            marker: PhantomData,
        }
    }
}

impl<E> Default for RuntimeBackgroundTaskRegistry<E> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeBackgroundTaskState::default())),
            history_limit: DEFAULT_TASK_HISTORY_LIMIT,
            marker: PhantomData,
        }
    }
}

impl<E> RuntimeBackgroundTaskRegistry<E> {
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

    pub(crate) fn cancel_all(&self) {
        let tokens = {
            let state = self.inner.lock();
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
                let token = state.controls.get(task_id).cloned().ok_or_else(|| {
                    RuntimeBackgroundTaskControlError::NotRunning(task_id.to_owned())
                })?;
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
                (task.clone(), token)
            };
        token.cancel();
        Ok(task)
    }

    pub(crate) fn spawn<F, Fut>(
        &self,
        spec: RuntimeBackgroundTaskSpec,
        work: F,
    ) -> RuntimeBackgroundTaskStart
    where
        E: Display + Send + 'static,
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<RuntimeBackgroundTaskOutcome, E>> + Send + 'static,
    {
        let (task, token, started) = {
            let mut state = self.inner.lock();
            if let Some(dedupe_key) = spec.dedupe_key()
                && let Some(existing_id) = state.active_by_key.get(dedupe_key)
                && let Some(existing) = state.tasks.get(existing_id)
                && existing.is_running()
            {
                return RuntimeBackgroundTaskStart {
                    started: false,
                    task: existing.clone(),
                };
            }

            let now = Utc::now();
            let task = RuntimeBackgroundTask {
                id: format!("rtask_{}", Uuid::new_v4().simple()),
                kind: spec.kind(),
                origin: spec.origin(),
                title: spec.title().to_owned(),
                status: RuntimeBackgroundTaskStatus::Running,
                message: None,
                error_message: None,
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
            (task, token, true)
        };

        if started {
            let registry = self.clone();
            let task_id = task.id.clone();
            tokio::spawn(async move {
                let completion = tokio::select! {
                    _ = token.cancelled() => RuntimeBackgroundTaskCompletion::Cancelled {
                        message: Some("Cancelled by operator.".to_owned()),
                    },
                    result = work(token.clone()) => match result {
                        Ok(RuntimeBackgroundTaskOutcome::Succeeded { message }) => {
                            RuntimeBackgroundTaskCompletion::Succeeded { message }
                        }
                        Ok(RuntimeBackgroundTaskOutcome::Cancelled { message }) => {
                            RuntimeBackgroundTaskCompletion::Cancelled { message }
                        }
                        Err(error) => RuntimeBackgroundTaskCompletion::Failed {
                            error_message: error.to_string(),
                        },
                    },
                };
                registry.finish(task_id.as_str(), completion);
            });
        }

        RuntimeBackgroundTaskStart { started, task }
    }

    fn finish(&self, task_id: &str, completion: RuntimeBackgroundTaskCompletion) {
        let mut state = self.inner.lock();
        if let Some(task) = state.tasks.get_mut(task_id) {
            task.finished_at = Some(Utc::now());
            match completion {
                RuntimeBackgroundTaskCompletion::Succeeded { message } => {
                    task.status = RuntimeBackgroundTaskStatus::Succeeded;
                    task.message = message;
                    task.error_message = None;
                }
                RuntimeBackgroundTaskCompletion::Failed { error_message } => {
                    task.status = RuntimeBackgroundTaskStatus::Failed;
                    task.message = None;
                    task.error_message = Some(error_message);
                }
                RuntimeBackgroundTaskCompletion::Cancelled { message } => {
                    task.status = RuntimeBackgroundTaskStatus::Cancelled;
                    task.message = message;
                    task.error_message = None;
                }
            }
        }

        state.controls.remove(task_id);
        if let Some(dedupe_key) = state.dedupe_keys.remove(task_id)
            && state
                .active_by_key
                .get(&dedupe_key)
                .is_some_and(|id| id == task_id)
        {
            state.active_by_key.remove(&dedupe_key);
        }
        state.trim_history(self.history_limit);
    }
}
