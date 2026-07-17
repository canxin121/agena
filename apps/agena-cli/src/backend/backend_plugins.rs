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
    command: &agena::plugin::PluginCommandDefinition,
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
        let mut tools = self
            .runtime
            .current_snapshot()
            .plugin_manager()
            .registered_tools()
            .into_iter()
            .map(|tool| {
                let mut tags = tool
                    .effective_tags()
                    .into_iter()
                    .map(|tag| tag.as_ref().to_string())
                    .collect::<Vec<_>>();
                tags.sort();
                tags.dedup();
                PermissionToolCatalogItem {
                    name: tool.canonical_name(),
                    summary: tool.summary_text().unwrap_or_default().to_string(),
                    tags,
                }
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools.dedup_by(|left, right| left.name == right.name);
        tools
    }

    pub fn plugin_statusline_segments(&self) -> Vec<agena::plugin::HostStatuslineSegment> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .statusline_segments()
    }

    pub fn plugin_tui_content_blocks(
        &self,
    ) -> Vec<agena::plugin::PluginTuiContentBlockCatalogItem> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .tui_content_blocks()
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

    pub fn plugin_theme_palettes(&self) -> Vec<agena::plugin::HostThemePalette> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .theme_palettes()
    }

    pub fn plugin_statuses(&self) -> Vec<agena::plugin::status::PluginStatus> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_statuses()
    }

    pub fn plugin_inspect(&self, plugin_id: &str) -> Option<agena::plugin::PluginInspect> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_inspect(plugin_id)
    }

    pub fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Vec<agena::plugin::PluginLogRecord> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
            .plugin_logs(plugin_id, after_seq, limit)
    }

    pub fn plugin_slash_commands(&self) -> Vec<agena::plugin::PluginCommandCatalogItem> {
        self.runtime
            .current_snapshot()
            .plugin_manager()
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
        match dispatch::dispatch_command(&self.app_state, ApiCommand::UpsertPermissionRule(params))
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
            &self.app_state,
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
            &self.app_state,
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

    pub fn snapshot_inspector_rows(&self) -> Vec<InspectorRow> {
        let Some(manager) = self.runtime.session_manager() else {
            return vec![InspectorRow {
                label: "session_runtime".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let executor = manager.tool_executor();
        let Some(registry) = executor.snapshot_registry() else {
            return vec![InspectorRow {
                label: "snapshot_registry".to_string(),
                detail: "unavailable".to_string(),
            }];
        };
        let active = tool::snapshot_list_active(registry);
        let managed = tool::snapshot_list_managed(&self.workspace_root, registry);
        let capabilities = tool::snapshot_backend_capabilities(&self.workspace_root);
        let mut rows = vec![
            InspectorRow {
                label: "preferred_backend".to_string(),
                detail: capabilities
                    .preferred_backend
                    .map(|backend| backend.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            },
            InspectorRow {
                label: "rift_backend".to_string(),
                detail: format!(
                    "available={} | {}",
                    capabilities.rift.available, capabilities.rift.detail
                ),
            },
            InspectorRow {
                label: "git_backend".to_string(),
                detail: format!(
                    "available={} | {}",
                    capabilities.git.available, capabilities.git.detail
                ),
            },
            InspectorRow {
                label: "active_sessions".to_string(),
                detail: active.len().to_string(),
            },
            InspectorRow {
                label: "managed_dirs".to_string(),
                detail: managed.len().to_string(),
            },
        ];
        rows.extend(active.into_iter().map(|entry| InspectorRow {
            label: format!("session #{}", entry.session_id),
            detail: format!(
                "{} | backend={} | branch={} | created_here={}",
                entry.path.display(),
                entry.backend,
                entry.branch,
                entry.created_here
            ),
        }));
        rows.extend(managed.into_iter().map(|entry| {
            let session_id = entry
                .session_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string());
            let branch = entry
                .branch
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let stale = entry.is_stale();
            InspectorRow {
                label: entry.path.display().to_string(),
                detail: format!(
                    "session={} | backend={} | branch={} | git_registered={} | rift_registered={} | stale={}",
                    session_id,
                    entry.backend
                        .map(|backend| backend.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    branch,
                    entry.registered_with_git,
                    entry.registered_with_rift,
                    stale
                ),
            }
        }));
        rows
    }

    pub fn enter_snapshot(
        &self,
        session_id: i64,
        name: Option<String>,
        path: Option<String>,
    ) -> Result<SnapshotCommandOutput> {
        let manager = self.session_manager()?;
        let output = manager
            .tool_executor()
            .execute_tool_payload_for_host(
                "enter_snapshot",
                serde_json::to_value(EnterSnapshotToolInput { name, path })?,
                Some(session_id),
                None,
                None,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        parse_snapshot_payload(output.payload)
    }

    pub fn exit_snapshot(
        &self,
        session_id: i64,
        action: String,
        discard_changes: bool,
    ) -> Result<SnapshotCommandOutput> {
        let manager = self.session_manager()?;
        let output = manager
            .tool_executor()
            .execute_tool_payload_for_host(
                "exit_snapshot",
                serde_json::to_value(ExitSnapshotToolInput {
                    action,
                    discard_changes,
                })?,
                Some(session_id),
                None,
                None,
            )
            .map_err(|error| anyhow!(error.to_string()))?;
        parse_snapshot_payload(output.payload)
    }

    pub async fn invoke_plugin_slash_command(
        &self,
        entry: &agena::plugin::PluginCommandCatalogItem,
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
                agena::plugin::PluginUiAction::None => return Ok(PluginCommandEffect::None),
                agena::plugin::PluginUiAction::SubmitPrompt { prompt } => {
                    return Ok(PluginCommandEffect::SubmitPrompt(prompt));
                }
                agena::plugin::PluginUiAction::OpenRoute { route } => {
                    return Ok(PluginCommandEffect::OpenRoute(route));
                }
                agena::plugin::PluginUiAction::OpenUrl { url } => {
                    return Ok(PluginCommandEffect::OpenUrl(url));
                }
                agena::plugin::PluginUiAction::InvokeTool {
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
                agena::plugin::PluginUiAction::InvokeCommand {
                    command,
                    input: base_input,
                } => {
                    let session_id = session_id.ok_or_else(|| {
                        anyhow!("plugin command invocation requires an active session")
                    })?;
                    let manager = self.session_manager()?;
                    let session = manager.get_session(session_id).await?;
                    let executor = manager
                        .tool_executor()
                        .for_session_context(&session.runtime().execution);
                    let permission_name = format!("plugin.command.{plugin_id}.{command}");
                    let check = agena::tool::ToolPermissionCheck {
                        action: agena::permission::tool_action(
                            permission_name.as_str(),
                            None,
                            &[],
                            Some(&executor.agent().tool_policy),
                        ),
                        decision: executor.agent().authorize_tool_names(
                            &[permission_name.as_str()],
                            None,
                            &[],
                        ),
                    };
                    match manager
                        .resolve_tool_permission_check(Some(session.id), &check)
                        .await?
                        .decision
                    {
                        agena::permission::PermissionDecision::Allow => {}
                        agena::permission::PermissionDecision::Ask { reason } => {
                            return Err(anyhow!(
                                "plugin command requires approval and was not executed: {reason}"
                            ));
                        }
                        agena::permission::PermissionDecision::Deny { reason } => {
                            return Err(anyhow!("plugin command denied: {reason}"));
                        }
                    }

                    let host = self.runtime.current_snapshot().plugin_manager();
                    let output = host
                        .invoke_plugin_command(
                            plugin_id.as_str(),
                            agena::plugin::PluginCommandInvokeInput {
                                session_id: Some(session_id),
                                call_id: None,
                                workspace_root: Some(
                                    self.workspace_root.to_string_lossy().into_owned(),
                                ),
                                command_id: command,
                                slash: slash.clone(),
                                raw: raw.to_string(),
                                input: merge_plugin_command_input(base_input, Some(input)),
                            },
                        )
                        .map_err(|error| anyhow!(error.to_string()))?;

                    match output {
                        agena::plugin::PluginCommandOutput::None => {
                            return Ok(PluginCommandEffect::None);
                        }
                        agena::plugin::PluginCommandOutput::Message { text } => {
                            return Ok(PluginCommandEffect::Message(text));
                        }
                        agena::plugin::PluginCommandOutput::SubmitPrompt { prompt } => {
                            return Ok(PluginCommandEffect::SubmitPrompt(prompt));
                        }
                        agena::plugin::PluginCommandOutput::OpenRoute { route } => {
                            return Ok(PluginCommandEffect::OpenRoute(route));
                        }
                        agena::plugin::PluginCommandOutput::OpenUrl { url } => {
                            return Ok(PluginCommandEffect::OpenUrl(url));
                        }
                        agena::plugin::PluginCommandOutput::InvokeTool {
                            tool,
                            input: next_input,
                            submit_output_as_prompt,
                        } => {
                            action = agena::plugin::PluginUiAction::InvokeTool {
                                tool,
                                input: next_input,
                                submit_output_as_prompt,
                            };
                            input = serde_json::json!({});
                        }
                        agena::plugin::PluginCommandOutput::InvokeCommand {
                            command,
                            input: next_input,
                        } => {
                            action = self
                                .runtime
                                .current_snapshot()
                                .plugin_manager()
                                .resolve_studio_action(plugin_id.as_str(), command.as_str())
                                .unwrap_or(agena::plugin::PluginUiAction::InvokeCommand {
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
        let manager = self.session_manager()?;
        let host = self.runtime.current_snapshot().plugin_manager();
        let entry = host
            .resolve_registered_tool_for_plugin_tool("agena.skills", "run")
            .ok_or_else(|| anyhow!("skills command renderer is unavailable"))?;
        let input = agena::message::StructuredObject::try_from(json!({
            "name": name,
            "args": args.trim(),
        }))
        .map_err(|error| anyhow!(error))?;
        let invocation =
            ToolInvocation::plugin_named(entry.canonical_name(), entry.plugin_full_name(), input);
        let execution = manager
            .tool_executor()
            .execute_invocation_detailed(&invocation, session_id, -1)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(execution.view.output_text)
    }

    async fn invoke_plugin_command_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<String> {
        let session_id = session_id
            .ok_or_else(|| anyhow!("plugin tool invocation requires an active session"))?;
        let manager = self.session_manager()?;
        let host = self.runtime.current_snapshot().plugin_manager();
        let entry = host
            .resolve_registered_tool_for_plugin_tool(plugin_id, tool_name)
            .ok_or_else(|| anyhow!("plugin tool not found: {plugin_id}/{tool_name}"))?;
        let input = match input {
            serde_json::Value::Null => serde_json::json!({}),
            serde_json::Value::Object(_) => input,
            other => return Err(anyhow!("plugin tool input must be an object, got {other}")),
        };
        let structured = agena::message::StructuredObject::try_from(input).map_err(|error| {
            anyhow!("invalid plugin tool input for {plugin_id}/{tool_name}: {error}")
        })?;
        let invocation = ToolInvocation::plugin_named(
            entry.canonical_name(),
            entry.plugin_full_name(),
            structured,
        );
        let execution = match manager
            .authorize_session_tool_invocation(session_id, invocation)
            .await?
        {
            agena::session::ToolInvocationAuthorization::Allowed(authorized) => authorized
                .execute(-1)
                .map_err(|error| anyhow!(error.to_string()))?,
            agena::session::ToolInvocationAuthorization::Ask { reason } => {
                return Err(anyhow!(
                    "plugin tool requires approval and was not executed: {reason}"
                ));
            }
            agena::session::ToolInvocationAuthorization::Deny { reason } => {
                return Err(anyhow!("plugin tool denied: {reason}"));
            }
        };
        Ok(execution.view.output_text)
    }

    pub async fn create_commit(&self, message: String) -> Result<(String, String)> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }
        if status.staged_files == 0 {
            return Err(anyhow!("no staged changes to commit"));
        }

        let output = Command::new("git")
            .args(["commit", "-m", message.as_str()])
            .current_dir(&self.workspace_root)
            .output()
            .context("failed to execute git commit")?;
        if !output.status.success() {
            return Err(anyhow!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let commit = git_command_output(&self.workspace_root, ["rev-parse", "HEAD"])?;
        let summary = git_command_output(&self.workspace_root, ["log", "-1", "--pretty=%s"])?;
        Ok((commit, summary))
    }

    pub async fn create_pr(
        &self,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<String> {
        let status = self
            .git_status()
            .await
            .context("failed to load git status")?;
        if !status.git_available {
            return Err(anyhow!("git is not available in PATH"));
        }
        if !status.gh_available {
            return Err(anyhow!("gh is not available in PATH"));
        }
        if !status.repo {
            return Err(anyhow!(
                "not a git repository: {}",
                self.workspace_root.display()
            ));
        }

        let branch = head
            .clone()
            .or(status.branch.clone())
            .ok_or_else(|| anyhow!("could not determine current branch"))?;

        let mut command = Command::new("gh");
        command.arg("pr").arg("create").arg("--title").arg(title);
        command.arg("--body").arg(body.unwrap_or_default());
        if let Some(base) = base {
            command.arg("--base").arg(base);
        }
        command.arg("--head").arg(branch);
        command.current_dir(&self.workspace_root);

        let output = command.output().context("failed to execute gh pr create")?;
        if !output.status.success() {
            return Err(anyhow!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub(super) async fn resolve_workspace_resource(
        &self,
        create_if_missing: bool,
    ) -> Result<WorkspaceResource> {
        match dispatch::dispatch_command(
            &self.app_state,
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

    pub(super) async fn git_status(&self) -> Result<GitStatusResource> {
        let workspace_root = self.runtime.workspace_root().to_path_buf();
        let git_available = agena::git::command_available("git");
        let gh_available = agena::git::command_available("gh");

        if self.runtime.session_manager().is_none() {
            return Ok(GitStatusResource {
                git_available,
                repo: false,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        if !git_available {
            return Ok(GitStatusResource {
                git_available,
                repo: false,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        let repo = agena::git::succeeds(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                git_available,
                repo,
                gh_available,
                branch: None,
                staged_files: 0,
            });
        }

        let branch = git_command_output(&workspace_root, ["branch", "--show-current"])?;
        let status = git_command_output(&workspace_root, ["status", "--porcelain"])?;
        let staged_files = agena::git::summarize_status(status.as_str()).staged;

        Ok(GitStatusResource {
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            staged_files,
        })
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
                &self.app_state,
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

    pub(super) fn session_manager(&self) -> Result<Arc<agena::session::SessionManager>> {
        self.runtime
            .session_manager()
            .ok_or_else(|| anyhow!("session runtime is not available"))
    }

    pub(super) fn memory_store(&self) -> MemoryStore {
        MemoryStore::for_workspace(&self.workspace_root)
    }
}
use crate::backend::Result;
use crate::backend::{
    ApiCommand, Arc, Backend, Command, CommandResult, EnterSnapshotToolInput,
    ExitSnapshotToolInput, GitStatusResource, InspectorRow, ListSessionsParams, MemoryStore, Path,
    PermissionRuleResource, PermissionToolCatalogItem, PluginCommandEffect, Query, QueryResult,
    ReplacePermissionRuleParams, SessionResource, SnapshotCommandOutput, ToolInvocation,
    UpsertPermissionRuleParams, WorkspaceResource, api_error, dispatch, git_command_output,
    non_empty, parse_snapshot_payload, tool,
};
