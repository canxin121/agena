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

    pub fn plugin_statusline_segments(&self) -> Vec<agena_plugin_host::HostStatuslineSegment> {
        self.application.plugin_runtime().statusline_segments()
    }

    pub fn plugin_tui_content_blocks(
        &self,
    ) -> Vec<agena_plugin_host::PluginTuiContentBlockCatalogItem> {
        self.application.plugin_runtime().tui_content_blocks()
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
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::UpsertPermissionRule(params),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to create permission rule")
    }

    pub async fn replace_permission_rule(
        &self,
        rule_id: i64,
        params: UpsertPermissionRuleParams,
    ) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::ReplacePermissionRule(ReplacePermissionRuleParams {
                rule_id,
                rule: params,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
        .context("failed to replace permission rule")
    }

    pub async fn revoke_permission_rule(&self, rule_id: i64) -> Result<PermissionRuleResource> {
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::RevokePermissionRule(agena_api::commands::RevokePermissionRuleParams {
                rule_id,
                reason: None,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::PermissionRule(rule) => Ok(rule),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
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
                            false,
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

    pub fn render_skill_prompt(&self, session_id: i64, name: &str, args: &str) -> Result<String> {
        let entry = self
            .application
            .plugin_runtime()
            .resolve_plugin_tool(Some("agena.skills"), "run")
            .ok_or_else(|| anyhow!("skills command renderer is unavailable"))?;
        let input = agena_domain::StructuredObject::try_from(json!({
            "name": name,
            "args": args.trim(),
        }))
        .map_err(|error| anyhow!(error))?;
        let invocation =
            ToolInvocation::plugin_named(entry.canonical_name, entry.plugin_full_name, input);
        self.application
            .session_execution_services()
            .map_err(|error| anyhow!(error.to_string()))?
            .tool_execution
            .render_session_tool_output(session_id, invocation)
            .map_err(|error| anyhow!(error.to_string()))
    }

    pub async fn invoke_plugin_workbench_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<String> {
        self.invoke_plugin_command_tool(plugin_id, tool_name, input, session_id, true)
            .await
    }

    async fn invoke_plugin_command_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        user_approved: bool,
    ) -> Result<String> {
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
        let invocation =
            ToolInvocation::plugin_named(entry.canonical_name, entry.plugin_full_name, structured);
        let tool_execution = self
            .application
            .session_execution_services()
            .map_err(|error| anyhow!(error.to_string()))?
            .tool_execution;
        let execution = if user_approved {
            tool_execution.execute_session_tool_with_user_approval(session_id, invocation)
        } else {
            tool_execution.execute_session_tool(session_id, invocation)
        };
        let summary = execution.await.map_err(|error| match error {
            agena_runtime::SessionToolExecutionError::ApprovalRequired(reason) => {
                anyhow!("plugin tool requires approval and was not executed: {reason}")
            }
            agena_runtime::SessionToolExecutionError::Denied(reason) => {
                anyhow!("plugin tool denied: {reason}")
            }
            agena_runtime::SessionToolExecutionError::Execution(error) => anyhow!(error),
        })?;
        Ok(summary.output_text)
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
        match dispatch::dispatch_command(
            &self.application,
            ApiCommand::ResolveWorkspace(agena_api::commands::ResolveWorkspaceParams {
                path: self.workspace_root.to_string_lossy().to_string(),
                create_if_missing,
            }),
        )
        .await
        .map_err(api_error)?
        {
            CommandResult::Workspace(workspace) => Ok(workspace),
            other => Err(anyhow!("unexpected command result: {:?}", other)),
        }
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
            let page = match dispatch::dispatch_query(
                &self.application,
                Query::ListSessions(ListSessionsParams {
                    cursor: cursor.clone(),
                    limit: Some(limit),
                    workspace_id: query.workspace_id,
                    parent_id: query.parent_id,
                    roots: query.roots,
                    search: query.search.clone(),
                }),
            )
            .await
            .map_err(api_error)?
            {
                QueryResult::Sessions(page) => page,
                other => return Err(anyhow!("unexpected query result: {:?}", other)),
            };
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
    ApiCommand, Backend, CommandResult, ListSessionsParams, Path, PermissionRuleResource,
    PermissionToolCatalogItem, PluginCommandEffect, Query, QueryResult,
    ReplacePermissionRuleParams, SessionResource, ToolInvocation, UpsertPermissionRuleParams,
    WorkspaceResource, api_error, dispatch,
};
