use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
