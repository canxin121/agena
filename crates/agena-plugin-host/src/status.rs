//! Plugin daemon lifecycle status registry.
//!
//! Tracks per-plugin runtime state so that operators (and other plugins, when
//! authorized via `HostCapability::PluginStatus`) can observe pid, restart
//! count, last exit code and last error for stdio-style daemons. Non-stdio
//! transports (`inproc`, `cdylib`, `http`, `wasm`) report `Running` with no
//! pid/restart fields populated.

use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize};

use crate::sdk::PluginKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRunState {
    Running,
    Restarting,
    Failed,
    Stopped,
}

impl AsRef<str> for PluginRunState {
    fn as_ref(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Restarting => "restarting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for PluginRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStatus {
    pub plugin_id: PluginKey,
    pub kind: &'static str,
    pub state: PluginRunState,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub last_restart_at_ms: Option<i64>,
    pub last_failure: Option<agena_failure::UserProblem>,
}

impl PluginStatus {
    pub fn initial(plugin_id: &PluginKey, kind: &'static str) -> Self {
        Self {
            plugin_id: plugin_id.clone(),
            kind,
            state: PluginRunState::Running,
            pid: None,
            restart_count: 0,
            last_exit_code: None,
            last_restart_at_ms: None,
            last_failure: None,
        }
    }

    pub fn key(&self) -> PluginKey {
        self.plugin_id.clone()
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Process-local registry shared between [`crate::host::PluginHost`] and the
/// transports that update lifecycle state.
#[derive(Debug, Default)]
pub struct StatusRegistry {
    inner: RwLock<HashMap<PluginKey, PluginStatus>>,
}

impl StatusRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn set(&self, status: PluginStatus) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(status.key(), status);
        }
    }

    pub fn remove(&self, plugin_id: &PluginKey) {
        if let Ok(mut guard) = self.inner.write() {
            guard.remove(plugin_id);
        }
    }

    pub fn get(&self, plugin_id: &PluginKey) -> Option<PluginStatus> {
        self.inner
            .read()
            .ok()
            .and_then(|guard| guard.get(plugin_id).cloned())
    }

    pub fn list(&self) -> Vec<PluginStatus> {
        let mut entries = self
            .inner
            .read()
            .map(|guard| guard.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        entries.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        entries
    }

    pub fn update<F>(&self, plugin_id: &PluginKey, mutator: F)
    where
        F: FnOnce(&mut PluginStatus),
    {
        if let Ok(mut guard) = self.inner.write()
            && let Some(status) = guard.get_mut(plugin_id)
        {
            mutator(status);
        }
    }

    pub fn record_started(&self, plugin_id: &PluginKey, pid: Option<u32>, is_restart: bool) {
        self.update(plugin_id, |status| {
            status.state = PluginRunState::Running;
            status.pid = pid;
            status.last_failure = None;
            if is_restart {
                status.restart_count = status.restart_count.saturating_add(1);
                status.last_restart_at_ms = Some(now_ms());
            }
        });
    }

    pub fn record_spawn_failure(&self, plugin_id: &PluginKey, diagnostic: impl fmt::Display) {
        let failure = plugin_lifecycle_failure(plugin_id, diagnostic);
        self.update(plugin_id, |status| {
            status.state = PluginRunState::Failed;
            status.last_failure = Some(failure);
            status.pid = None;
        });
    }

    pub fn record_exit(
        &self,
        plugin_id: &PluginKey,
        will_restart: bool,
        exit_code: Option<i32>,
        message: Option<String>,
    ) {
        let failure = message.map(|diagnostic| plugin_lifecycle_failure(plugin_id, diagnostic));
        self.update(plugin_id, |status| {
            status.state = if will_restart {
                PluginRunState::Restarting
            } else {
                PluginRunState::Failed
            };
            status.pid = None;
            if let Some(code) = exit_code {
                status.last_exit_code = Some(code);
            }
            if let Some(failure) = failure {
                status.last_failure = Some(failure);
            }
        });
    }

    pub fn record_stopped(&self, plugin_id: &PluginKey) {
        self.update(plugin_id, |status| {
            status.state = PluginRunState::Stopped;
            status.pid = None;
        });
    }
}

fn plugin_lifecycle_failure(
    plugin_id: &PluginKey,
    diagnostic: impl fmt::Display,
) -> agena_failure::UserProblem {
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new("plugin.lifecycle_failed"),
        agena_failure::FailureCategory::DependencyUnavailable,
        agena_failure::FailureResponsibility::Dependency,
        agena_failure::RetryDirective::Backoff,
        agena_failure::RecoveryDirective::RestartPlugin,
        agena_failure::FailureImpact::RuntimeDegraded,
        agena_failure::UserPresentation::new(
            "plugin-lifecycle-failed",
            "The plugin stopped unexpectedly. Restart the plugin and try again.",
        ),
    );
    tracing::error!(
        plugin_id = %plugin_id,
        failure_id = %failure.id,
        diagnostic = %diagnostic,
        "plugin lifecycle failure"
    );
    failure.into()
}

#[cfg(test)]
mod tests {
    use super::{PluginStatus, StatusRegistry};
    use crate::sdk::PluginKey;

    #[test]
    fn plugin_diagnostic_never_enters_status_projection() {
        let plugin_id = PluginKey::new("example", "unsafe").expect("plugin key");
        let registry = StatusRegistry::new();
        registry.set(PluginStatus::initial(&plugin_id, "stdio"));
        registry.record_spawn_failure(
            &plugin_id,
            "transport failed token=secret /private/plugin.sock ANSI=\u{1b}[31m",
        );

        let status = registry.get(&plugin_id).expect("plugin status");
        let public = serde_json::to_string(&status).expect("serialize status");
        assert!(status.last_failure.is_some());
        assert!(public.contains("plugin stopped unexpectedly"));
        assert!(!public.contains("token=secret"));
        assert!(!public.contains("/private/plugin.sock"));
        assert!(!public.contains("transport failed"));
    }
}
