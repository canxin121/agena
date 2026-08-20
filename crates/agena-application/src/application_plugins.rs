//! Plugin-facing operations, migrated from
//! `agena-tui-backend/src/backend_plugins.rs` (`activity_kind_catalog`,
//! `invoke_plugin_tool`, and the private `invoke_plugin_tool_checked`
//! helper).

use agena_plugin_host::sdk::{
    PluginHostEffect, PluginOperationDiagnostic, PluginOperationInvokeInput, PluginOperationResult,
    PluginOperationStatus, PluginOperationTarget,
};
use agena_plugin_host::{PluginToolInvokeResponse, PluginToolInvokeStatus};

use crate::{Application, ApplicationError};

impl Application {
    pub fn plugin_settings(
        &self,
        plugin_id: &str,
    ) -> Result<crate::dto::PluginSettingsResponse, ApplicationError> {
        let inspect = self
            .plugin_runtime()
            .plugin_inspect(plugin_id)
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The plugin was not found.",
                    format!("plugin not found: {plugin_id}"),
                )
            })?;
        let manifest = inspect.manifest.ok_or_else(|| {
            ApplicationError::service_unavailable(format!(
                "plugin `{plugin_id}` has no active manifest"
            ))
        })?;
        let contract = manifest.settings.ok_or_else(|| {
            ApplicationError::not_found_with_diagnostic(
                "This plugin does not expose editable settings.",
                format!("plugin settings contract unavailable: {plugin_id}"),
            )
        })?;
        let defaults = contract.default_value().map_err(|error| {
            ApplicationError::internal(format!(
                "plugin `{plugin_id}` has an invalid settings default: {error}"
            ))
        })?;
        let configured = inspect
            .configured_plugin
            .map(|configured| configured.settings)
            .unwrap_or(serde_json::Value::Null);
        let mut effective = defaults.clone();
        if !configured.is_null() {
            merge_plugin_settings(&mut effective, &configured);
        }

        fn merge_plugin_settings(target: &mut serde_json::Value, configured: &serde_json::Value) {
            match (target, configured) {
                (serde_json::Value::Object(target), serde_json::Value::Object(configured)) => {
                    for (key, value) in configured {
                        match target.get_mut(key) {
                            Some(existing) => merge_plugin_settings(existing, value),
                            None => {
                                target.insert(key.clone(), value.clone());
                            }
                        }
                    }
                }
                (target, configured) => *target = configured.clone(),
            }
        }
        let diagnostics = contract
            .validate_value(&effective)
            .err()
            .map(|error| {
                vec![crate::dto::PluginSettingsDiagnostic {
                    path: error.path.unwrap_or_default(),
                    message: error.message,
                }]
            })
            .unwrap_or_default();
        Ok(crate::dto::PluginSettingsResponse {
            plugin_id: plugin_id.to_owned(),
            contract,
            defaults,
            configured,
            effective,
            diagnostics,
        })
    }

    pub async fn update_plugin_settings(
        &self,
        plugin_id: &str,
        value: serde_json::Value,
    ) -> Result<crate::dto::PluginSettingsUpdateResponse, ApplicationError> {
        let current = self.plugin_settings(plugin_id)?;
        current.contract.validate_value(&value).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic(
                "The plugin settings are invalid.",
                error.to_string(),
            )
        })?;
        let edit = self
            .set_plugin_settings_setting(plugin_id, &[], value)
            .await?;
        Ok(crate::dto::PluginSettingsUpdateResponse {
            settings: self.plugin_settings(plugin_id)?,
            reload_required: edit.reload_required,
        })
    }

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

    pub async fn invoke_plugin_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<PluginToolInvokeResponse, ApplicationError> {
        self.invoke_plugin_tool_checked(plugin_id, tool_name, input, session_id)
            .await
    }

    async fn invoke_plugin_tool_checked(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<PluginToolInvokeResponse, ApplicationError> {
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
                PluginToolInvokeStatus::Completed,
                summary.title,
                summary.output_text,
                summary.payload,
                summary.metadata,
            ),
            agena_runtime::SessionToolExecutionOutcome::CapabilityUnavailable(unavailable) => (
                PluginToolInvokeStatus::CapabilityUnavailable,
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
                PluginToolInvokeStatus::ToolUnavailable,
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
        Ok(PluginToolInvokeResponse {
            plugin_id: entry.plugin_id,
            tool: entry.canonical_name,
            status,
            title,
            output_text,
            payload,
            metadata,
        })
    }

    /// Resolve, validate and execute one user-facing plugin operation. Tool
    /// targets stay on the normal session/permission execution path; method
    /// targets cross the plugin transport. No client follows an action chain.
    pub async fn invoke_plugin_operation(
        &self,
        plugin_id: &str,
        operation_id: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        workspace_root: Option<String>,
        slash: Option<String>,
        raw: String,
    ) -> Result<PluginOperationResult, ApplicationError> {
        let entry = self
            .plugin_runtime()
            .operation_catalog()
            .into_iter()
            .find(|entry| {
                entry.plugin_id.to_string() == plugin_id && entry.operation.id == operation_id
            })
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The plugin operation was not found.",
                    format!("plugin operation not found: {plugin_id}/{operation_id}"),
                )
            })?;

        let input_is_empty =
            input.is_null() || input.as_object().is_some_and(serde_json::Map::is_empty);
        let input = if input_is_empty && !raw.trim().is_empty() {
            entry.operation.input.parse_shorthand(raw.as_str())
        } else if input_is_empty {
            entry.operation.input.default_value()
        } else {
            entry.operation.input.validate_value(&input).map(|()| input)
        }
        .map_err(|error| {
            ApplicationError::bad_request_with_diagnostic(
                "The plugin operation input is invalid.",
                error.to_string(),
            )
        })?;

        let mut result = match &entry.operation.target {
            PluginOperationTarget::Method { .. } => self
                .plugin_runtime()
                .invoke_plugin_operation(
                    plugin_id,
                    PluginOperationInvokeInput {
                        operation_id: operation_id.to_owned(),
                        input,
                        session_id,
                        call_id: None,
                        workspace_root,
                        slash,
                        raw,
                    },
                )
                .await
                .map_err(ApplicationError::internal)?,
            PluginOperationTarget::Tool { tool } => {
                let tool_result = self
                    .invoke_plugin_tool(plugin_id, tool.as_str(), input, session_id)
                    .await?;
                plugin_operation_result_from_tool(plugin_id, tool_result)
            }
        };
        if result.title.trim().is_empty() {
            result.title = entry.operation.title;
        }
        result.validate().map_err(|error| {
            ApplicationError::internal(format!(
                "plugin operation `{plugin_id}/{operation_id}` returned an invalid result: {error}"
            ))
        })?;
        Ok(result)
    }
}

fn plugin_operation_result_from_tool(
    plugin_id: &str,
    tool: PluginToolInvokeResponse,
) -> PluginOperationResult {
    let status = match tool.status {
        PluginToolInvokeStatus::Completed => PluginOperationStatus::Succeeded,
        PluginToolInvokeStatus::CapabilityUnavailable | PluginToolInvokeStatus::ToolUnavailable => {
            PluginOperationStatus::Unavailable
        }
    };
    let retryable = tool
        .payload
        .as_ref()
        .and_then(|payload| payload.get("retryable"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let diagnostics = if status == PluginOperationStatus::Succeeded {
        Vec::new()
    } else {
        vec![PluginOperationDiagnostic {
            code: tool
                .payload
                .as_ref()
                .and_then(|payload| payload.get("code"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool_unavailable")
                .to_owned(),
            message: tool.output_text.clone(),
            path: None,
            sensitive: false,
        }]
    };
    PluginOperationResult {
        status,
        title: tool.title,
        summary: tool
            .output_text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Completed")
            .chars()
            .take(160)
            .collect(),
        detail: (!tool.output_text.trim().is_empty()).then_some(tool.output_text),
        output: tool.payload,
        diagnostics,
        retryable,
        effects: vec![PluginHostEffect::RefreshPluginSurface {
            plugin_id: plugin_id.to_owned(),
        }],
    }
}
