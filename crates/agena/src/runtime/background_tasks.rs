use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::Arc,
};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::AppError;

const DEFAULT_TASK_HISTORY_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackgroundTaskKind {
    ModelCatalogRefresh,
    RuntimeReload,
    MarketplaceRegistrySync,
    MarketplacePluginInstall,
    MarketplacePluginUninstall,
    MarketplacePluginUpgrade,
}

impl RuntimeBackgroundTaskKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::ModelCatalogRefresh => "Refresh model catalog",
            Self::RuntimeReload => "Reload runtime",
            Self::MarketplaceRegistrySync => "Sync marketplace registry",
            Self::MarketplacePluginInstall => "Install marketplace plugin",
            Self::MarketplacePluginUninstall => "Uninstall marketplace plugin",
            Self::MarketplacePluginUpgrade => "Upgrade marketplace plugin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackgroundTaskOrigin {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackgroundTaskStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBackgroundTask {
    pub id: String,
    pub kind: RuntimeBackgroundTaskKind,
    pub origin: RuntimeBackgroundTaskOrigin,
    pub title: String,
    pub status: RuntimeBackgroundTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub cancellable: bool,
}

impl RuntimeBackgroundTask {
    pub fn is_running(&self) -> bool {
        self.status == RuntimeBackgroundTaskStatus::Running
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeBackgroundTaskStart {
    pub started: bool,
    pub task: RuntimeBackgroundTask,
}

#[derive(Debug, Clone)]
pub enum RuntimeBackgroundTaskOutcome {
    Succeeded { message: Option<String> },
    Cancelled { message: Option<String> },
}

impl RuntimeBackgroundTaskOutcome {
    pub fn succeeded(message: impl Into<String>) -> Self {
        Self::Succeeded {
            message: Some(message.into()),
        }
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::Cancelled {
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeBackgroundTaskControlError {
    #[error("runtime is shutting down")]
    Shutdown,
    #[error("background task `{0}` not found")]
    NotFound(String),
    #[error("background task `{0}` is not running")]
    NotRunning(String),
    #[error("background task `{0}` cannot be cancelled")]
    NotCancellable(String),
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeBackgroundTaskSpec {
    kind: RuntimeBackgroundTaskKind,
    origin: RuntimeBackgroundTaskOrigin,
    title: String,
    dedupe_key: Option<String>,
    cancellable: bool,
}

impl RuntimeBackgroundTaskSpec {
    pub(crate) fn new(
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Self {
        Self {
            kind,
            origin,
            title: kind.title().to_owned(),
            dedupe_key: None,
            cancellable: true,
        }
    }

    pub(crate) fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub(crate) fn with_dedupe_key(mut self, dedupe_key: impl Into<String>) -> Self {
        self.dedupe_key = Some(dedupe_key.into());
        self
    }

    pub(crate) fn with_cancellable(mut self, cancellable: bool) -> Self {
        self.cancellable = cancellable;
        self
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeBackgroundTaskRegistry {
    inner: Arc<Mutex<RuntimeBackgroundTaskRegistryState>>,
    history_limit: usize,
}

impl Default for RuntimeBackgroundTaskRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeBackgroundTaskRegistryState::default())),
            history_limit: DEFAULT_TASK_HISTORY_LIMIT,
        }
    }
}

impl RuntimeBackgroundTaskRegistry {
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
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<RuntimeBackgroundTaskOutcome, AppError>> + Send + 'static,
    {
        let (task, token, started) = {
            let mut state = self.inner.lock();
            if let Some(dedupe_key) = spec.dedupe_key.as_deref()
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
                kind: spec.kind,
                origin: spec.origin,
                title: spec.title,
                status: RuntimeBackgroundTaskStatus::Running,
                message: None,
                error_message: None,
                created_at: now,
                started_at: now,
                finished_at: None,
                cancellable: spec.cancellable,
            };
            let token = CancellationToken::new();
            state.order.push_front(task.id.clone());
            state.tasks.insert(task.id.clone(), task.clone());
            state.controls.insert(task.id.clone(), token.clone());
            if let Some(dedupe_key) = spec.dedupe_key {
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

#[derive(Debug)]
enum RuntimeBackgroundTaskCompletion {
    Succeeded { message: Option<String> },
    Failed { error_message: String },
    Cancelled { message: Option<String> },
}

#[derive(Default)]
struct RuntimeBackgroundTaskRegistryState {
    order: VecDeque<String>,
    tasks: HashMap<String, RuntimeBackgroundTask>,
    controls: HashMap<String, CancellationToken>,
    active_by_key: HashMap<String, String>,
    dedupe_keys: HashMap<String, String>,
}

impl RuntimeBackgroundTaskRegistryState {
    fn trim_history(&mut self, history_limit: usize) {
        if self.order.len() <= history_limit {
            return;
        }

        let mut index = self.order.len();
        while self.order.len() > history_limit && index > 0 {
            index -= 1;
            let Some(task_id) = self.order.get(index).cloned() else {
                break;
            };
            let should_remove = self
                .tasks
                .get(task_id.as_str())
                .map(|task| !task.is_running())
                .unwrap_or(true);
            if !should_remove {
                continue;
            }

            let _ = self.order.remove(index);
            self.tasks.remove(task_id.as_str());
            self.controls.remove(task_id.as_str());
            if let Some(dedupe_key) = self.dedupe_keys.remove(task_id.as_str())
                && self
                    .active_by_key
                    .get(dedupe_key.as_str())
                    .is_some_and(|id| id == task_id.as_str())
            {
                self.active_by_key.remove(dedupe_key.as_str());
            }
        }
    }
}
