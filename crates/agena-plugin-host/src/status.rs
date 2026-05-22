//! Plugin daemon lifecycle status registry.
//!
//! Tracks per-plugin runtime state so that operators (and other plugins, when
//! authorized via `HostCapability::PluginStatus`) can observe pid, restart
//! count, last exit code and last error for stdio-style daemons. Non-stdio
//! transports (`inproc`, `cdylib`, `http`, `wasm`) report `Running` with no
//! pid/restart fields populated.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRunState {
    Running,
    Restarting,
    Failed,
    Stopped,
}

impl PluginRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Restarting => "restarting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStatus {
    pub plugin_id: String,
    pub kind: &'static str,
    pub state: PluginRunState,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub last_restart_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

impl PluginStatus {
    pub fn initial(plugin_id: impl Into<String>, kind: &'static str) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            kind,
            state: PluginRunState::Running,
            pid: None,
            restart_count: 0,
            last_exit_code: None,
            last_restart_at_ms: None,
            last_error: None,
        }
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
    inner: RwLock<HashMap<String, PluginStatus>>,
}

impl StatusRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn set(&self, status: PluginStatus) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(status.plugin_id.clone(), status);
        }
    }

    pub fn remove(&self, plugin_id: &str) {
        if let Ok(mut guard) = self.inner.write() {
            guard.remove(plugin_id);
        }
    }

    pub fn get(&self, plugin_id: &str) -> Option<PluginStatus> {
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

    pub fn update<F>(&self, plugin_id: &str, mutator: F)
    where
        F: FnOnce(&mut PluginStatus),
    {
        if let Ok(mut guard) = self.inner.write()
            && let Some(status) = guard.get_mut(plugin_id)
        {
            mutator(status);
        }
    }

    pub fn record_started(&self, plugin_id: &str, pid: Option<u32>, is_restart: bool) {
        self.update(plugin_id, |status| {
            status.state = PluginRunState::Running;
            status.pid = pid;
            status.last_error = None;
            if is_restart {
                status.restart_count = status.restart_count.saturating_add(1);
                status.last_restart_at_ms = Some(now_ms());
            }
        });
    }

    pub fn record_spawn_failure(&self, plugin_id: &str, message: impl Into<String>) {
        self.update(plugin_id, |status| {
            status.state = PluginRunState::Failed;
            status.last_error = Some(message.into());
            status.pid = None;
        });
    }

    pub fn record_exit(
        &self,
        plugin_id: &str,
        will_restart: bool,
        exit_code: Option<i32>,
        message: Option<String>,
    ) {
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
            if let Some(message) = message {
                status.last_error = Some(message);
            }
        });
    }

    pub fn record_stopped(&self, plugin_id: &str) {
        self.update(plugin_id, |status| {
            status.state = PluginRunState::Stopped;
            status.pid = None;
        });
    }
}
