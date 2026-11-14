//! Plugin administration projections and command context resolution.
use super::wire;
use crate::{Application, ApplicationError};
use agena_api::management::{PluginCommand, PluginQuery};
use serde_json::Value;

impl Application {
    pub async fn query_plugin_management(
        &self,
        query: PluginQuery,
    ) -> Result<Value, ApplicationError> {
        let runtime = self.plugin_runtime();
        match query {
            PluginQuery::List => {
                let items = runtime
                    .plugin_statuses()
                    .into_iter()
                    .map(|status| {
                        let manifest = runtime
                            .plugin_inspect(status.plugin_id.to_string().as_str())
                            .and_then(|plugin| plugin.manifest);
                        crate::dto::PluginListItem::new(status, manifest)
                    })
                    .collect::<Vec<_>>();
                Ok(serde_json::json!({ "items": items }))
            }
            PluginQuery::Get { plugin_id } => {
                let plugin = runtime.plugin_inspect(&plugin_id).ok_or_else(|| {
                    ApplicationError::not_found_with_diagnostic(
                        "The plugin was not found.",
                        format!("plugin not found: {plugin_id}"),
                    )
                })?;
                wire(crate::dto::PluginInspectResponse { plugin })
            }
            PluginQuery::Settings { plugin_id } => wire(self.plugin_settings(&plugin_id)?),
            PluginQuery::Architecture => wire(runtime.plugin_architecture_catalog()),
            PluginQuery::Surface => wire(crate::dto::PluginSurfaceCatalogResponse {
                catalog: runtime.plugin_surface_catalog(),
                permission_tools: runtime
                    .permission_tool_catalog()
                    .into_iter()
                    .map(|tool| crate::dto::PermissionToolCatalogResource {
                        name: tool.name,
                        summary: tool.summary,
                        summary_translations: tool.summary_translations,
                        tags: tool.tags,
                    })
                    .collect(),
                notifications: runtime.host_notifications(),
                notifications_at_ms: Some(chrono::Utc::now().timestamp_millis()),
                activity_kinds: self.activity_kind_catalog(),
                tool_registry_generation: runtime.tool_registry_generation(),
                tool_registry_last_event: runtime
                    .tool_registry_events_since(None, 1)
                    .into_iter()
                    .next(),
            }),
            PluginQuery::ToolChanges {
                after_generation,
                limit,
            } => Ok(serde_json::json!({
                "generation": runtime.tool_registry_generation(),
                "events": runtime.tool_registry_events_since(after_generation, limit.unwrap_or(100)),
            })),
            PluginQuery::Logs {
                plugin_id,
                after_seq,
                limit,
            } => {
                if !runtime
                    .plugin_statuses()
                    .iter()
                    .any(|status| status.plugin_id.to_string() == plugin_id)
                {
                    return Err(ApplicationError::not_found_with_diagnostic(
                        "The plugin was not found.",
                        format!("plugin not found: {plugin_id}"),
                    ));
                }
                wire(crate::dto::PluginLogListResponse {
                    plugin_id: plugin_id.clone(),
                    logs: runtime.plugin_logs(&plugin_id, after_seq, limit.unwrap_or(50)),
                })
            }
            PluginQuery::WebSettings {
                plugin_id,
                page_id,
                generation,
            } => wire(self.plugin_web_settings(&plugin_id, &page_id, &generation)?),
        }
    }

    pub async fn command_plugin_management(
        &self,
        command: PluginCommand,
    ) -> Result<Value, ApplicationError> {
        match command {
            PluginCommand::SetSettings {
                plugin_id,
                settings,
            } => wire(self.update_plugin_settings(&plugin_id, settings).await?),
            PluginCommand::InvokeTool {
                plugin_id,
                tool_id,
                input,
                session_id,
            } => wire(
                self.invoke_plugin_tool(&plugin_id, &tool_id, input, session_id)
                    .await?,
            ),
            PluginCommand::InvokeCommand {
                plugin_id,
                command_id,
                input,
                session_id,
                slash,
                raw,
            } => {
                let workspace_root = self.plugin_command_workspace_root(session_id, true).await?;
                let result = self
                    .invoke_plugin_command(
                        &plugin_id,
                        &command_id,
                        input,
                        session_id,
                        workspace_root,
                        slash,
                        raw,
                    )
                    .await?;
                wire(agena_api::resource::PluginCommandInvokeResponse {
                    plugin_id,
                    command_id,
                    result,
                })
            }
            PluginCommand::SetWebSettings {
                plugin_id,
                page_id,
                generation,
                settings,
            } => wire(
                self.update_plugin_web_settings(&plugin_id, &page_id, &generation, settings)
                    .await?,
            ),
            PluginCommand::InvokeWebCommand {
                plugin_id,
                page_id,
                generation,
                command_id,
                input,
                session_id,
            } => {
                let workspace_root = self
                    .plugin_command_workspace_root(session_id, false)
                    .await?;
                let result = self
                    .invoke_plugin_web_command(
                        &plugin_id,
                        &page_id,
                        &generation,
                        agena_plugin_host::sdk::CommandInvokeInput {
                            command_id: command_id.clone(),
                            input,
                            session_id,
                            call_id: None,
                            workspace_root,
                            slash: None,
                            raw: String::new(),
                        },
                    )
                    .await?;
                wire(agena_api::resource::PluginCommandInvokeResponse {
                    plugin_id,
                    command_id,
                    result,
                })
            }
        }
    }

    pub async fn plugin_command_workspace_root(
        &self,
        session_id: Option<i64>,
        allow_bot: bool,
    ) -> Result<Option<String>, ApplicationError> {
        let Some(session_id) = session_id else {
            return Ok(None);
        };
        let session = self
            .service()
            .get_session(session_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The session was not found.",
                    format!("session not found: {session_id}"),
                )
            })?;
        if allow_bot && session.owner_bot_id.is_some() {
            return Ok(None);
        }
        let workspace_id = command_workspace_id(session.workspace_id)?;
        let workspace = self
            .service()
            .get_workspace(workspace_id)
            .await?
            .ok_or_else(|| {
                ApplicationError::not_found_with_diagnostic(
                    "The workspace was not found.",
                    format!("workspace not found: {workspace_id}"),
                )
            })?;
        Ok(Some(workspace.path))
    }
}

fn command_workspace_id(workspace_id: Option<i64>) -> Result<i64, ApplicationError> {
    workspace_id.ok_or_else(|| ApplicationError::bad_request(
        "This session has no project workspace. Run the plugin command from a session in an explicit project."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_command_requires_an_explicit_project_session() {
        assert_eq!(command_workspace_id(Some(42)).unwrap(), 42);
        let error = command_workspace_id(None).unwrap_err();
        assert!(
            serde_json::to_string(error.failure.as_ref())
                .unwrap()
                .contains("explicit project")
        );
    }
}
