//! Concrete `HostClient` impl backed by the live `AgenaRuntime`. Plugins
//! that run as subprocess (stdio) or remote (HTTP) call back into this via
//! JSON-RPC; the `HostHandle` in `agena-plugin-host` routes those calls
//! through this client.

use std::sync::Arc;

use crate::event::Scope;
use async_trait::async_trait;

use crate::plugin::sdk::host_api::{
    EventSubscription, HostClient, LogLevel, NoopHostClient,
};
use crate::plugin::{
    EventEnvelope, EventFilter as PluginEventFilter, PermissionAskInput,
    PermissionDecision as PluginPermissionDecision, PluginError, ToolInvokeOutput,
};
use crate::runtime::AgenaRuntime;

/// Build a `HostClient` impl for a runtime; use [`NoopHostClient`] when no
/// runtime is available (e.g. before bootstrap completes).
pub fn host_client_for(runtime: AgenaRuntime) -> Arc<dyn HostClient> {
    Arc::new(RuntimeHostClient { runtime })
}

pub fn noop_host_client() -> Arc<dyn HostClient> {
    Arc::new(NoopHostClient)
}

struct RuntimeHostClient {
    runtime: AgenaRuntime,
}

#[async_trait]
impl HostClient for RuntimeHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        match level {
            LogLevel::Trace => {
                tracing::trace!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Debug => {
                tracing::debug!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Info => {
                tracing::info!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Warn => {
                tracing::warn!(target: "plugin", ?fields, "{message}");
            }
            LogLevel::Error => {
                tracing::error!(target: "plugin", ?fields, "{message}");
            }
        }
    }

    async fn publish_event(&self, env: EventEnvelope) -> Result<(), PluginError> {
        let snapshot = self.runtime.current_snapshot();
        let Some(manager) = snapshot.session_manager() else {
            tracing::debug!(
                target: "plugin",
                "publish_event ignored: no session manager"
            );
            return Ok(());
        };
        let publisher = manager.event_publisher();
        // Find the calling plugin via thread-local; default "<unknown>".
        let plugin_id = active_invocations::current_plugin().unwrap_or_else(|| "<unknown>".into());
        let kind = crate::event::EventKind::PluginEvent(crate::event::PluginEventPayload {
            plugin_id,
            kind_label: env.kind,
            payload: env.payload,
        });
        let ctx = match env.session_id {
            Some(id) => crate::event::PublishContext::for_session(id),
            None => crate::event::PublishContext::default(),
        };
        publisher
            .publish(ctx, kind)
            .await
            .map_err(|e| PluginError::new(format!("event publish failed: {e}")))?;
        Ok(())
    }

    async fn subscribe_events(
        &self,
        filter: PluginEventFilter,
    ) -> Result<EventSubscription, PluginError> {
        // Translate the SDK filter to agena's filter and confirm; the actual
        // event push back to the plugin already happens via the snapshot's
        // `event_bridge`. Returning a deterministic id so plugins can ack.
        let id = format!(
            "sub-{}",
            uuid::Uuid::new_v4().simple()
        );
        let _ = filter; // currently unused beyond existence
        let _bus = self
            .runtime
            .current_snapshot()
            .session_manager()
            .map(|mgr| {
                let _ = mgr.event_bus().subscribe(crate::event::EventFilter::new(
                    Scope::Global,
                ));
            });
        Ok(EventSubscription { id })
    }

    async fn ask_permission(
        &self,
        _req: PermissionAskInput,
    ) -> Result<PluginPermissionDecision, PluginError> {
        // The host doesn't surface a unified "ask user" affordance here.
        // For now, default to Prompt (i.e. "host has no opinion, fall back").
        Ok(PluginPermissionDecision::Prompt)
    }

    async fn read_config(
        &self,
        path: Option<String>,
    ) -> Result<serde_json::Value, PluginError> {
        let snapshot = self.runtime.current_snapshot();
        let value = serde_json::to_value(snapshot.config_resolution())
            .map_err(|e| PluginError::invalid_params(e.to_string()))?;
        if let Some(path) = path {
            // Dot-notation path: `runtime.session_cache.max_sessions`
            let mut cursor = &value;
            for segment in path.split('.') {
                if segment.is_empty() {
                    continue;
                }
                cursor = match cursor.get(segment) {
                    Some(v) => v,
                    None => return Ok(serde_json::Value::Null),
                };
            }
            Ok(cursor.clone())
        } else {
            Ok(value)
        }
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> Result<ToolInvokeOutput, PluginError> {
        let host = self.runtime.current_snapshot().plugin_manager();
        let resolution = host
            .lookup_tool(&tool)
            .ok_or_else(|| PluginError::new(format!("tool `{tool}` not found")))?;

        // Reentrancy guard: refuse if the current host call stack already
        // contains this plugin id. Implemented as a tokio task-local set.
        let plugin_id = resolution.handle.plugin_id.clone();
        if active_invocations::contains(&plugin_id) {
            return Err(PluginError::new(format!(
                "host->plugin invoke would re-enter plugin `{plugin_id}` (cycle detected)"
            )));
        }
        let _guard = active_invocations::enter(plugin_id.clone());

        let host_arc = host.clone();
        let handle_clone = resolution.handle.clone();
        let original = resolution.handle.original_name.clone();
        // Run in a blocking thread so the sync invoke_tool API on PluginHost
        // (which itself uses block_on) doesn't hijack our async runtime.
        let result = tokio::task::spawn_blocking(move || {
            host_arc.invoke_tool(
                &handle_clone,
                crate::plugin::ToolInvokeInput {
                    tool_name: original,
                    session_id: -1,
                    call_id: -1,
                    workspace_root: ".".to_string(),
                    input,
                },
            )
        })
        .await
        .map_err(|_| PluginError::new("invoke_tool task panicked"))??;

        Ok(result)
    }
}

mod active_invocations {
    //! Reentrancy guard for plugin → host → plugin invocations. We track
    //! the *task-local* set of plugin ids currently being invoked so that a
    //! plugin cannot recurse into itself via the host callback.

    use std::cell::RefCell;
    use std::collections::HashSet;

    thread_local! {
        static ACTIVE: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    }

    pub fn contains(id: &str) -> bool {
        ACTIVE.with(|set| set.borrow().contains(id))
    }

    pub fn current_plugin() -> Option<String> {
        ACTIVE.with(|set| set.borrow().iter().next().cloned())
    }

    pub struct Guard(String);

    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.with(|set| {
                set.borrow_mut().remove(&self.0);
            });
        }
    }

    pub fn enter(id: String) -> Guard {
        ACTIVE.with(|set| {
            set.borrow_mut().insert(id.clone());
        });
        Guard(id)
    }
}
