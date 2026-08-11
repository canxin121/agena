use anyhow::{Context, anyhow};
use serde_json::json;

fn merge_plugin_command_input(
    base: Option<serde_json::Value>,
    overlay: Option<serde_json::Value>,
) -> serde_json::Value {
    match (base, overlay) {
        (Some(serde_json::Value::Object(mut base)), Some(serde_json::Value::Object(overlay))) => {
            base.extend(overlay);
            serde_json::Value::Object(base)
        }
        (_, Some(value)) => value,
        (Some(value), None) => value,
        (None, None) => json!({}),
    }
}

fn parse_plugin_command_literal(
    raw: &str,
    schema: Option<&serde_json::Value>,
) -> serde_json::Value {
    let parsed =
        serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.into()));
    let Some(expected) = schema
        .and_then(serde_json::Value::as_object)
        .and_then(|schema| schema.get("type"))
    else {
        return parsed;
    };
    let matches_type = |kind: &str| match kind {
        "string" => parsed.is_string(),
        "integer" => parsed.as_i64().is_some() || parsed.as_u64().is_some(),
        "number" => parsed.is_number(),
        "boolean" => parsed.is_boolean(),
        "object" => parsed.is_object(),
        "array" => parsed.is_array(),
        "null" => parsed.is_null(),
        _ => true,
    };
    let accepted = match expected {
        serde_json::Value::String(kind) => matches_type(kind),
        serde_json::Value::Array(kinds) => kinds
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(matches_type),
        _ => true,
    };
    if accepted {
        parsed
    } else {
        serde_json::Value::String(raw.into())
    }
}

fn plugin_command_input(
    command: &agena_plugin_host::PluginCommandDefinition,
    raw: &str,
) -> Result<serde_json::Value> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(json!({}));
    }
    let Some(schema) = command.input_schema.as_ref() else {
        return Ok(json!({ "args": raw }));
    };
    let schema_object = schema.as_object();
    let schema_type = schema_object
        .and_then(|schema| schema.get("type"))
        .and_then(serde_json::Value::as_str);
    let properties = schema_object
        .and_then(|schema| schema.get("properties"))
        .and_then(serde_json::Value::as_object);

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
        if schema_type != Some("object") || parsed.is_object() {
            return Ok(parsed);
        }
        if let Some(properties) = properties
            && properties.len() == 1
        {
            let (name, _) = properties.iter().next().expect("one command property");
            return Ok(json!({ (name): parsed }));
        }
    }

    if schema_type == Some("object")
        && let Some(properties) = properties
    {
        if properties.len() == 1 {
            let (name, property_schema) = properties.iter().next().expect("one command property");
            return Ok(json!({
                (name): parse_plugin_command_literal(raw, Some(property_schema)),
            }));
        }

        let mut aliases = std::collections::HashMap::<&str, &str>::new();
        for (name, property_schema) in properties {
            if let Some(values) = property_schema
                .get("x-agena-aliases")
                .and_then(serde_json::Value::as_array)
            {
                for alias in values.iter().filter_map(serde_json::Value::as_str) {
                    aliases.insert(alias, name.as_str());
                }
            }
        }
        let mut output = serde_json::Map::new();
        for token in raw.split_whitespace() {
            let Some((raw_name, value)) = token.split_once('=') else {
                output.clear();
                break;
            };
            let name = if properties.contains_key(raw_name) {
                raw_name
            } else if let Some(name) = aliases.get(raw_name) {
                name
            } else {
                output.clear();
                break;
            };
            output.insert(
                name.to_string(),
                parse_plugin_command_literal(value, properties.get(name)),
            );
        }
        if !output.is_empty() {
            return Ok(serde_json::Value::Object(output));
        }
    }

    let literal = parse_plugin_command_literal(raw, Some(schema));
    if !literal.is_string() || schema_type == Some("string") {
        return Ok(literal);
    }
    Ok(json!({ "args": raw }))
}

impl Backend {
    pub fn permission_tool_catalog(&self) -> Vec<PermissionToolCatalogItem> {
        self.application
            .plugin_runtime()
            .permission_tool_catalog()
            .into_iter()
            .map(|tool| PermissionToolCatalogItem {
                name: tool.name,
                summary: tool.summary,
                tags: tool.tags,
            })
            .collect()
    }

    pub fn plugin_display_contributions(&self) -> Vec<agena_plugin_host::HostDisplayContribution> {
        self.application.plugin_runtime().display_contributions()
    }

    /// Re-publish the plan progress display contribution for `session_id`.
    ///
    /// The composer's bottom-right plan chip is backed by an in-memory
    /// display contribution that starts empty after a process restart or a
    /// runtime reload. Invoking `agena.plan.get` re-syncs the contribution
    /// from durable storage (the planning plugin re-publishes on every plan
    /// read) without mutating the plan. Returns `true` when the session has
    /// an active plan, so the caller can back off for sessions without one.
    pub async fn refresh_plan_display(&self, session_id: i64) -> Result<bool> {
        let response = self
            .invoke_plugin_ui_tool(
                "agena.plan",
                "get",
                serde_json::json!({ "view": "summary" }),
                Some(session_id),
            )
            .await?;
        Ok(response
            .payload
            .as_ref()
            .and_then(|payload| payload.get("plan"))
            .is_some_and(|plan| !plan.is_null()))
    }

    /// Plugin notifications emitted through the unified `host.notify` entry
    /// (Phase 6). Bounded recent queue; the TUI dedupes/consumes each intent.
    pub fn plugin_host_notifications(&self) -> Vec<agena_plugin_host::HostNotification> {
        self.application.plugin_runtime().host_notifications()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn workspace_name(&self) -> String {
        self.workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.workspace_root.display().to_string())
    }

    pub fn plugin_theme_palettes(&self) -> Vec<agena_plugin_host::HostThemePalette> {
        self.application.plugin_runtime().theme_palettes()
    }

    pub fn plugin_statuses(&self) -> Vec<agena_plugin_host::status::PluginStatus> {
        self.application.plugin_runtime().plugin_statuses()
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<agena_plugin_host::PluginInspect> {
        self.application.plugin_runtime().plugin_inspect(plugin_id)
    }

    /// Dynamic activity-kind catalog: built-in kinds merged with every kind
    /// declared by a loaded plugin manifest. New plugins automatically
    /// contribute their kinds to transcript expansion settings.
    pub fn activity_kind_catalog(&self) -> Vec<agena_domain::ActivityKind> {
        let mut kinds = agena_domain::builtin_activity_kinds();
        for status in self.plugin_statuses() {
            let Some(inspect) = self.plugin_inspect(&status.plugin_id.to_string()) else {
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

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena_plugin_host::PluginLogRecord> {
        self.application
            .plugin_runtime()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    pub fn plugin_slash_commands(&self) -> Vec<agena_plugin_host::PluginCommandCatalogItem> {
        self.application
            .plugin_runtime()
            .studio_commands()
            .into_iter()
            .filter(|entry| {
                entry
                    .command
                    .slash
                    .as_deref()
                    .is_some_and(|slash| !slash.trim().trim_start_matches('/').is_empty())
            })
            .collect()
    }

    pub async fn create_permission_rule(
        &self,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        let UpsertPermissionRuleParams {
            action_key,
            subject_kind,
            tool_name,
            qualifier,
            path_access_kind,
            workspace_root,
            target_path,
            network_target,
            network_host,
            network_port,
            scope,
            session_id,
            mode,
        } = params;
        self.application
            .service()
            .create_permission_rule(agena_application::dto::PermissionRuleWriteRequest {
                action_key,
                subject_kind,
                tool_name,
                qualifier,
                path_access_kind,
                workspace_root,
                target_path,
                network_target,
                network_host,
                network_port,
                scope,
                session_id,
                mode,
            })
            .await
            .map_err(anyhow::Error::new)
            .context("failed to create permission rule")
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        let UpsertPermissionRuleParams {
            action_key,
            subject_kind,
            tool_name,
            qualifier,
            path_access_kind,
            workspace_root,
            target_path,
            network_target,
            network_host,
            network_port,
            scope,
            session_id,
            mode,
        } = params;
        self.application
            .service()
            .replace_permission_rule(
                rule_id,
                agena_application::dto::PermissionRuleWriteRequest {
                    action_key,
                    subject_kind,
                    tool_name,
                    qualifier,
                    path_access_kind,
                    workspace_root,
                    target_path,
                    network_target,
                    network_host,
                    network_port,
                    scope,
                    session_id,
                    mode,
                },
            )
            .await
            .map_err(anyhow::Error::new)
            .context("failed to replace permission rule")
    }

    pub async fn revoke_permission_rule(&self, rule_id: i64) -> Result<PermissionRuleResource> {
        self.application
            .service()
            .revoke_permission_rule(rule_id, None)
            .await
            .map_err(anyhow::Error::new)
            .context("failed to revoke permission rule")
    }

    pub async fn invoke_plugin_slash_command(
        &self,
        entry: &agena_plugin_host::PluginCommandCatalogItem,
        session_id: Option<i64>,
        raw: &str,
    ) -> Result<PluginCommandEffect> {
        const MAX_COMMAND_DEPTH: usize = 8;

        let plugin_id = entry.plugin_id.to_string();
        let slash = entry.command.slash.clone();
        let mut action = entry.command.action.clone();
        let mut input = plugin_command_input(&entry.command, raw)?;
        let mut depth = 0usize;

        loop {
            if depth > MAX_COMMAND_DEPTH {
                return Err(anyhow!("plugin command recursion limit exceeded"));
            }

            match action {
                agena_plugin_host::PluginUiAction::None => return Ok(PluginCommandEffect::None),
                agena_plugin_host::PluginUiAction::SubmitPrompt { prompt } => {
                    return Ok(PluginCommandEffect::SubmitPrompt(prompt));
                }
                agena_plugin_host::PluginUiAction::OpenPluginWorkbench { tab } => {
                    return Ok(PluginCommandEffect::OpenPluginWorkbench { plugin_id, tab });
                }
                agena_plugin_host::PluginUiAction::OpenUrl { url } => {
                    return Ok(PluginCommandEffect::OpenUrl(url));
                }
                agena_plugin_host::PluginUiAction::InvokeTool {
                    tool,
                    input: base_input,
                    submit_output_as_prompt,
                } => {
                    let output = self
                        .invoke_plugin_command_tool(
                            plugin_id.as_str(),
                            tool.as_str(),
                            merge_plugin_command_input(base_input, Some(input)),
                            session_id,
                        )
                        .await?;
                    if output.trim().is_empty() {
                        return Ok(PluginCommandEffect::None);
                    }
                    return if submit_output_as_prompt {
                        Ok(PluginCommandEffect::SubmitPrompt(output))
                    } else {
                        Ok(PluginCommandEffect::Message(output))
                    };
                }
                agena_plugin_host::PluginUiAction::InvokeCommand {
                    command,
                    input: base_input,
                } => {
                    let session_id = session_id.ok_or_else(|| {
                        anyhow!("plugin command invocation requires an active session")
                    })?;
                    let output = self
                        .application
                        .session_execution_services()
                        .map_err(|error| anyhow!(error.to_string()))?
                        .plugin_commands
                        .invoke_session_plugin_command(agena_runtime::SessionPluginCommandRequest {
                            session_id,
                            plugin_id: plugin_id.clone(),
                            command_id: command.clone(),
                            input: merge_plugin_command_input(base_input, Some(input)),
                            slash: slash.clone(),
                            raw: raw.to_string(),
                            workspace_root: Some(
                                self.workspace_root.to_string_lossy().into_owned(),
                            ),
                        })
                        .await
                        .map_err(|error| anyhow!(error.to_string()))?;

                    match output {
                        agena_plugin_host::PluginCommandOutput::None => {
                            return Ok(PluginCommandEffect::None);
                        }
                        agena_plugin_host::PluginCommandOutput::Message { text } => {
                            return Ok(PluginCommandEffect::Message(text));
                        }
                        agena_plugin_host::PluginCommandOutput::SubmitPrompt { prompt } => {
                            return Ok(PluginCommandEffect::SubmitPrompt(prompt));
                        }
                        agena_plugin_host::PluginCommandOutput::OpenPluginWorkbench { tab } => {
                            return Ok(PluginCommandEffect::OpenPluginWorkbench { plugin_id, tab });
                        }
                        agena_plugin_host::PluginCommandOutput::OpenUrl { url } => {
                            return Ok(PluginCommandEffect::OpenUrl(url));
                        }
                        agena_plugin_host::PluginCommandOutput::InvokeTool {
                            tool,
                            input: next_input,
                            submit_output_as_prompt,
                        } => {
                            action = agena_plugin_host::PluginUiAction::InvokeTool {
                                tool,
                                input: next_input,
                                submit_output_as_prompt,
                            };
                            input = serde_json::json!({});
                        }
                        agena_plugin_host::PluginCommandOutput::InvokeCommand {
                            command,
                            input: next_input,
                        } => {
                            action = self
                                .application
                                .plugin_runtime()
                                .resolve_studio_action(plugin_id.as_str(), command.as_str())
                                .unwrap_or(agena_plugin_host::PluginUiAction::InvokeCommand {
                                    command,
                                    input: next_input.clone(),
                                });
                            input = next_input.unwrap_or_else(|| serde_json::json!({}));
                        }
                    }
                    depth += 1;
                }
            }
        }
    }

    pub async fn invoke_plugin_workbench_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<String> {
        Ok(self
            .invoke_plugin_ui_tool(plugin_id, tool_name, input, session_id)
            .await?
            .output_text)
    }

    /// Invoke a plugin Tool API endpoint from a user-driven TUI surface.
    ///
    /// Unlike a slash command, callers need the structured payload as well as
    /// the human-readable output. This is used for read-only catalog flows
    /// such as the Skill picker; the selected Skill is later stored as an
    /// immutable message snapshot, never as a session activation.
    pub async fn invoke_plugin_ui_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<agena_plugin_host::PluginUiToolInvokeResponse> {
        self.invoke_plugin_ui_tool_checked(plugin_id, tool_name, input, session_id)
            .await
    }

    async fn invoke_plugin_command_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<String> {
        Ok(self
            .invoke_plugin_ui_tool_checked(plugin_id, tool_name, input, session_id)
            .await?
            .output_text)
    }

    async fn invoke_plugin_ui_tool_checked(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<agena_plugin_host::PluginUiToolInvokeResponse> {
        let session_id = session_id
            .ok_or_else(|| anyhow!("plugin tool invocation requires an active session"))?;
        let entry = self
            .application
            .plugin_runtime()
            .resolve_plugin_tool(Some(plugin_id), tool_name)
            .ok_or_else(|| anyhow!("plugin tool not found: {plugin_id}/{tool_name}"))?;
        let input = match input {
            serde_json::Value::Null => serde_json::json!({}),
            serde_json::Value::Object(_) => input,
            other => return Err(anyhow!("plugin tool input must be an object, got {other}")),
        };
        let structured = agena_domain::StructuredObject::try_from(input).map_err(|error| {
            anyhow!("invalid plugin tool input for {plugin_id}/{tool_name}: {error}")
        })?;
        let invocation = ToolInvocation::plugin_named(
            entry.canonical_name.clone(),
            entry.plugin_full_name,
            structured,
        );
        let tool_execution = self
            .application
            .session_execution_services()
            .map_err(|error| anyhow!(error.to_string()))?
            .tool_execution;
        let outcome = tool_execution
            .execute_session_tool(session_id, invocation)
            .await
            .map_err(|error| match error {
                agena_runtime::SessionToolExecutionError::Execution(error) => anyhow!(error),
            })?;
        let (status, title, output_text, payload, metadata) = match outcome {
            agena_runtime::SessionToolExecutionOutcome::Completed(summary) => (
                agena_plugin_host::PluginUiToolInvokeStatus::Completed,
                summary.title,
                summary.output_text,
                summary.payload,
                summary.metadata,
            ),
            agena_runtime::SessionToolExecutionOutcome::CapabilityUnavailable(unavailable) => (
                agena_plugin_host::PluginUiToolInvokeStatus::CapabilityUnavailable,
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
                agena_plugin_host::PluginUiToolInvokeStatus::ToolUnavailable,
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
        Ok(agena_plugin_host::PluginUiToolInvokeResponse {
            plugin_id: entry.plugin_id,
            tool: entry.canonical_name,
            status,
            title,
            output_text,
            payload,
            metadata,
        })
    }

    pub async fn create_commit(&self, message: String) -> Result<(String, String)> {
        let commit = self
            .application
            .git_commit(agena_application::dto::GitCommitRequest { message })
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok((commit.commit, commit.summary))
    }

    pub async fn create_pr(
        &self,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<String> {
        let pull_request = self
            .application
            .git_create_pull_request(agena_application::dto::GitPullRequestCreateRequest {
                title,
                body,
                base,
                head,
            })
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(pull_request.url)
    }

    pub(super) async fn resolve_workspace_resource(
        &self,
        create_if_missing: bool,
    ) -> Result<WorkspaceResource> {
        self.application
            .service()
            .resolve_workspace(agena_application::dto::WorkspaceResolveRequest {
                workspace: agena_application::dto::WorkspacePathRequest {
                    path: self.workspace_root.to_string_lossy().to_string(),
                },
                create_if_missing,
            })
            .await
            .map_err(anyhow::Error::new)
            .context("failed to resolve workspace")
    }

    pub(super) async fn current_workspace_id(&self) -> Result<i64> {
        Ok(self
            .resolve_workspace_resource(true)
            .await
            .context("failed to resolve current workspace")?
            .id)
    }

    pub(super) async fn list_sessions_query(
        &self,
        query: ListSessionsParams,
    ) -> Result<Vec<SessionResource>> {
        let mut cursor = query.cursor.clone();
        let limit = query.limit.unwrap_or(200);
        let mut items = Vec::new();

        loop {
            let page = self
                .application
                .service()
                .list_sessions(agena_application::dto::SessionListQuery {
                    pagination: agena_application::dto::SearchPaginationQuery {
                        pagination: agena_application::dto::CursorPaginationQuery {
                            cursor: cursor.clone(),
                            limit: Some(limit),
                        },
                        search: query.search.clone(),
                    },
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                })
                .await
                .map_err(anyhow::Error::new)?;
            cursor = page.page.next_cursor.clone();
            items.extend(page.items);
            if !page.page.has_more || cursor.is_none() {
                break;
            }
        }

        Ok(items)
    }

    pub(super) async fn resolve_session_root(&self, session_id: i64) -> Result<SessionResource> {
        let mut current = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        while let Some(parent_id) = current.parent_id {
            current = self
                .get_session(parent_id)
                .await?
                .ok_or_else(|| anyhow!("session not found: {parent_id}"))?;
        }
        Ok(current)
    }
}
use crate::Result;
use crate::{
    Backend, ListSessionsParams, Path, PermissionRuleResource, PermissionToolCatalogItem,
    PluginCommandEffect, SessionResource, ToolInvocation, UpsertPermissionRuleParams,
    WorkspaceResource,
};
