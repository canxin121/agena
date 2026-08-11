//! Plugin-facing operations, migrated from
//! `agena-tui-backend/src/backend_plugins.rs` (`activity_kind_catalog`,
//! `invoke_plugin_ui_tool`, and the private `invoke_plugin_ui_tool_checked`
//! helper).

use agena_plugin_host::{PluginUiToolInvokeResponse, PluginUiToolInvokeStatus};

use crate::{Application, ApplicationError};

impl Application {
    /// Dynamic activity-kind catalog: built-in kinds merged with every kind
    /// declared by a loaded plugin manifest. New plugins automatically
    /// contribute their kinds to transcript expansion settings.
    pub fn activity_kind_catalog(&self) -> Vec<agena_domain::ActivityKind> {
        let mut kinds = agena_domain::builtin_activity_kinds();
        for status in self.plugin_runtime().plugin_statuses() {
            let Some(inspect) = self
                .plugin_runtime()
                .plugin_inspect(&status.plugin_id.to_string())
            else {
                continue;
            };
            let Some(manifest) = inspect.manifest.as_ref() else {
                continue;
            };
            for kind in &manifest.activity_kinds {
                if !kinds.iter().any(|existing| existing.id == kind.id) {
                    kinds.push(kind.clone());
                }
            }
        }
        kinds
    }

    pub async fn invoke_plugin_ui_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<PluginUiToolInvokeResponse, ApplicationError> {
        self.invoke_plugin_ui_tool_checked(plugin_id, tool_name, input, session_id)
            .await
    }

    async fn invoke_plugin_ui_tool_checked(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<PluginUiToolInvokeResponse, ApplicationError> {
        let session_id = session_id.ok_or_else(|| {
            ApplicationError::internal("plugin tool invocation requires an active session")
        })?;
        let entry = self
            .plugin_runtime()
            .resolve_plugin_tool(Some(plugin_id), tool_name)
            .ok_or_else(|| {
                ApplicationError::internal(format!(
                    "plugin tool not found: {plugin_id}/{tool_name}"
                ))
            })?;
        let input = match input {
            serde_json::Value::Null => serde_json::json!({}),
            serde_json::Value::Object(_) => input,
            other => {
                return Err(ApplicationError::internal(format!(
                    "plugin tool input must be an object, got {other}"
                )));
            }
        };
        let structured = agena_domain::StructuredObject::try_from(input).map_err(|error| {
            ApplicationError::internal(format!(
                "invalid plugin tool input for {plugin_id}/{tool_name}: {error}"
            ))
        })?;
        let invocation = agena_domain::ToolInvocation::plugin_named(
            entry.canonical_name.clone(),
            entry.plugin_full_name,
            structured,
        );
        let tool_execution = self.session_execution_services()?.tool_execution;
        let outcome = tool_execution
            .execute_session_tool(session_id, invocation)
            .await
            .map_err(|error| match error {
                agena_runtime::SessionToolExecutionError::Execution(error) => {
                    ApplicationError::internal(error)
                }
            })?;
        let (status, title, output_text, payload, metadata) = match outcome {
            agena_runtime::SessionToolExecutionOutcome::Completed(summary) => (
                PluginUiToolInvokeStatus::Completed,
                summary.title,
                summary.output_text,
                summary.payload,
                summary.metadata,
            ),
            agena_runtime::SessionToolExecutionOutcome::CapabilityUnavailable(unavailable) => (
                PluginUiToolInvokeStatus::CapabilityUnavailable,
                "Capability unavailable".to_string(),
                format!(
                    "The operation was not executed because the current runtime does not provide the required capability: {}",
                    unavailable.reason
                ),
                Some(serde_json::json!({
                    "status": "capability_unavailable",
                    "code": "capability_unavailable",
                    "retryable": unavailable.retryable,
                    "unavailable": unavailable,
                })),
                Default::default(),
            ),
            agena_runtime::SessionToolExecutionOutcome::ToolUnavailable(unavailable) => (
                PluginUiToolInvokeStatus::ToolUnavailable,
                "Tool unavailable".to_string(),
                format!(
                    "The operation was not executed because the requested tool is unavailable: {}",
                    unavailable.reason
                ),
                Some(serde_json::json!({
                    "status": "tool_unavailable",
                    "code": "tool_unavailable",
                    "retryable": unavailable.retryable,
                    "unavailable": unavailable,
                })),
                Default::default(),
            ),
        };
        Ok(PluginUiToolInvokeResponse {
            plugin_id: entry.plugin_id,
            tool: entry.canonical_name,
            status,
            title,
            output_text,
            payload,
            metadata,
        })
    }
}
